//! Records over a unix stream socket, with descriptors beside them.
//!
//! Two thin calls over `sendmsg` and `recvmsg`: bytes go as any stream's,
//! and descriptors ride as `SCM_RIGHTS` on the first byte of a send. A
//! region's descriptor is passed this way, once ([`crate::region`]).
//! Callers feed received bytes to a [`crate::Decoder`].

use std::io;
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
use std::os::unix::net::UnixStream;

/// The most descriptors one send or receive carries.
pub const MAX_FDS: usize = 8;

/// Bytes of one descriptor, and of the most that go at once.
const FD: u32 = 4;
const FD_BYTES: u32 = 8 * FD;
const _: () = assert!(FD as usize == std::mem::size_of::<libc::c_int>() && FD_BYTES as usize == MAX_FDS * FD as usize);

/// Room for `MAX_FDS` descriptors in a control message, aligned as cmsghdr.
#[repr(C)]
union Control {
    _align: libc::cmsghdr,
    bytes: [u8; 64],
}

// SAFETY: CMSG_SPACE is arithmetic on its argument.
const _: () = assert!(unsafe { libc::CMSG_SPACE(FD_BYTES) } as usize <= std::mem::size_of::<Control>());

/// Send some of BYTES, and FDS with their first byte. Returns how many
/// bytes went; the descriptors went if any byte did. On a nonblocking
/// socket this fails with `WouldBlock` instead of waiting.
///
/// # Errors
/// `InvalidInput` for descriptors with no byte or more than [`MAX_FDS`],
/// else the socket's own error.
pub fn send(socket: &UnixStream, bytes: &[u8], fds: &[BorrowedFd<'_>]) -> io::Result<usize> {
    let count = u32::try_from(fds.len()).unwrap_or(u32::MAX);
    if count > FD_BYTES / FD || (bytes.is_empty() && count > 0) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "descriptors need a byte to ride on, and at most eight go at once"));
    }
    let mut iov = libc::iovec { iov_base: bytes.as_ptr().cast_mut().cast(), iov_len: bytes.len() };
    let mut control = Control { bytes: [0; 64] };
    // SAFETY: a zeroed msghdr is a valid empty one; its fields are set below.
    let mut message: libc::msghdr = unsafe { std::mem::zeroed() };
    message.msg_iov = &raw mut iov;
    message.msg_iovlen = 1;
    if !fds.is_empty() {
        let len = count * FD;
        // SAFETY: CONTROL is 64 zeroed bytes aligned for cmsghdr, enough
        // for CMSG_SPACE of eight descriptors (asserted above), so the
        // header and LEN data bytes that CMSG_DATA points at lie inside
        // it. BorrowedFd is a transparent c_int, so FDS is LEN bytes of
        // descriptors.
        unsafe {
            message.msg_control = control.bytes.as_mut_ptr().cast();
            message.msg_controllen = libc::CMSG_SPACE(len) as _;
            let header = libc::CMSG_FIRSTHDR(&raw const message);
            (*header).cmsg_level = libc::SOL_SOCKET;
            (*header).cmsg_type = libc::SCM_RIGHTS;
            (*header).cmsg_len = libc::CMSG_LEN(len) as _;
            std::ptr::copy_nonoverlapping(fds.as_ptr().cast::<u8>(), libc::CMSG_DATA(header), len as usize);
        }
    }
    loop {
        // SAFETY: MESSAGE points at IOV (BYTES, only read) and at CONTROL,
        // both alive across the call; the socket is open.
        let sent = unsafe { libc::sendmsg(socket.as_raw_fd(), &raw const message, libc::MSG_NOSIGNAL) };
        if let Ok(sent) = usize::try_from(sent) {
            return Ok(sent);
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

/// Send all of BYTES on a blocking socket, FDS with the first of them.
///
/// # Errors
/// As [`send`]; `WriteZero` when the socket takes nothing.
pub fn send_all(socket: &UnixStream, mut bytes: &[u8], mut fds: &[BorrowedFd<'_>]) -> io::Result<()> {
    while !bytes.is_empty() {
        let sent = send(socket, bytes, fds)?;
        if sent == 0 {
            return Err(io::ErrorKind::WriteZero.into());
        }
        (bytes, fds) = (&bytes[sent..], &[]);
    }
    Ok(())
}

/// Receive into BUFFER, pushing any descriptors that came onto FDS, each
/// close-on-exec and owned. Returns the byte count; 0 is the peer's end.
///
/// # Errors
/// The socket's own error; `WouldBlock` on a nonblocking socket with
/// nothing to read; `InvalidData` when the peer sent more descriptors than
/// fit, after which the connection is unusable: FDS gains none of that
/// receive's descriptors.
pub fn receive(socket: &UnixStream, buffer: &mut [u8], fds: &mut Vec<OwnedFd>) -> io::Result<usize> {
    let mut iov = libc::iovec { iov_base: buffer.as_mut_ptr().cast(), iov_len: buffer.len() };
    let mut control = Control { bytes: [0; 64] };
    // SAFETY: a zeroed msghdr is a valid empty one; its fields are set below.
    let mut message: libc::msghdr = unsafe { std::mem::zeroed() };
    message.msg_iov = &raw mut iov;
    message.msg_iovlen = 1;
    // SAFETY: taking the address of the union's byte view, which any bit
    // pattern inhabits.
    message.msg_control = unsafe { control.bytes.as_mut_ptr() }.cast();
    message.msg_controllen = std::mem::size_of::<Control>() as _;
    let received = loop {
        // SAFETY: MESSAGE points at IOV (BUFFER, exclusively borrowed) and
        // at CONTROL, both alive across the call and of the stated sizes.
        let received = unsafe { libc::recvmsg(socket.as_raw_fd(), &raw mut message, libc::MSG_CMSG_CLOEXEC) };
        if let Ok(received) = usize::try_from(received) {
            break received;
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    };
    let mut arrived = Vec::new();
    // SAFETY: the kernel filled CONTROL up to msg_controllen with whole
    // control messages; the CMSG macros walk only that. Each SCM_RIGHTS
    // payload is descriptors the kernel just installed in this process for
    // this call, owned by no one yet, so each becomes one OwnedFd. They
    // are read unaligned, as CMSG_DATA promises no alignment.
    unsafe {
        let mut header = libc::CMSG_FIRSTHDR(&raw const message);
        while !header.is_null() {
            if (*header).cmsg_level == libc::SOL_SOCKET && (*header).cmsg_type == libc::SCM_RIGHTS {
                let data = libc::CMSG_DATA(header);
                let count = ((*header).cmsg_len as usize - (data as usize - header as usize)) / FD as usize;
                for index in 0..count {
                    let fd = data.add(index * FD as usize).cast::<libc::c_int>().read_unaligned();
                    arrived.push(OwnedFd::from_raw_fd(fd));
                }
            }
            header = libc::CMSG_NXTHDR(&raw const message, header);
        }
    }
    // Some descriptors were discarded: which ones the records meant is lost.
    if message.msg_flags & libc::MSG_CTRUNC != 0 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "the peer sent more descriptors than one receive carries"));
    }
    fds.append(&mut arrived);
    Ok(received)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Decoder, Record, RegionReader, RegionWriter};
    use std::os::fd::AsFd;

    #[test]
    fn records_and_a_region_cross_a_socket() {
        let (ours, theirs) = UnixStream::pair().unwrap();
        let mut writer = RegionWriter::create(4096).unwrap();
        let span = writer.put(b"by shared memory").unwrap();
        let record = Record { seq: 1, class: 2, kind: 3, payload: span.write(crate::Writer::default()).finish() };
        let mut wire = Vec::new();
        record.encode_into(&mut wire);
        send_all(&ours, &wire, &[writer.fd()]).unwrap();

        let (mut decoder, mut fds, mut buffer) = (Decoder::default(), Vec::new(), [0; 256]);
        let count = receive(&theirs, &mut buffer, &mut fds).unwrap();
        decoder.push(&buffer[..count]);
        let received = decoder.pop().unwrap().unwrap();
        assert_eq!(received, record);
        assert_eq!(fds.len(), 1);
        let reader = RegionReader::open(fds.pop().unwrap().as_fd()).unwrap();
        let span = crate::Span::read(&mut crate::Reader(&received.payload)).unwrap();
        assert_eq!(reader.copy(span).unwrap(), b"by shared memory");
    }

    #[test]
    fn the_peers_end_reads_as_zero() {
        let (ours, theirs) = UnixStream::pair().unwrap();
        drop(ours);
        assert_eq!(receive(&theirs, &mut [0; 8], &mut Vec::new()).unwrap(), 0);
    }

    #[test]
    fn a_nonblocking_socket_reports_would_block() {
        let (ours, theirs) = UnixStream::pair().unwrap();
        theirs.set_nonblocking(true).unwrap();
        assert_eq!(receive(&theirs, &mut [0; 8], &mut Vec::new()).unwrap_err().kind(), io::ErrorKind::WouldBlock);
        ours.set_nonblocking(true).unwrap();
        let block = vec![0u8; 1 << 16];
        let error = loop {
            match send(&ours, &block, &[]) { Ok(_) => {}, Err(error) => break error }
        };
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
    }

    #[test]
    fn received_descriptors_are_close_on_exec() {
        let (ours, theirs) = UnixStream::pair().unwrap();
        send_all(&ours, b"x", &[ours.as_fd(), theirs.as_fd()]).unwrap();
        let mut fds = Vec::new();
        receive(&theirs, &mut [0; 8], &mut fds).unwrap();
        assert_eq!(fds.len(), 2);
        for fd in &fds {
            // SAFETY: F_GETFD reads flags of an open descriptor.
            let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFD) };
            assert!(flags & libc::FD_CLOEXEC != 0);
        }
    }

    /// Credentials beside the descriptors overflow the control buffer, as
    /// a peer sending too many descriptors would.
    #[test]
    fn truncated_descriptors_are_an_error_and_none_is_kept() {
        let (ours, theirs) = UnixStream::pair().unwrap();
        let on: libc::c_int = 1;
        // SAFETY: SO_PASSCRED takes a pointer to an int of the stated size.
        let set = unsafe { libc::setsockopt(theirs.as_raw_fd(), libc::SOL_SOCKET, libc::SO_PASSCRED, (&raw const on).cast(), FD) };
        assert_eq!(set, 0);
        send_all(&ours, b"x", &[ours.as_fd(); MAX_FDS]).unwrap();
        let mut fds = Vec::new();
        assert_eq!(receive(&theirs, &mut [0; 8], &mut fds).unwrap_err().kind(), io::ErrorKind::InvalidData);
        assert!(fds.is_empty());
    }

    #[test]
    fn descriptors_without_a_byte_are_refused() {
        let (ours, _theirs) = UnixStream::pair().unwrap();
        assert_eq!(send(&ours, b"", &[ours.as_fd()]).unwrap_err().kind(), io::ErrorKind::InvalidInput);
    }
}
