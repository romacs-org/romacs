//! Records over a unix stream socket, with descriptors beside them.
//!
//! Two thin calls over `sendmsg` and `recvmsg`: bytes go as any stream's,
//! and descriptors ride as `SCM_RIGHTS` on the first byte of a send. A
//! region's descriptor is passed this way, once ([`crate::region`]).
//! Callers feed received bytes to a [`crate::Decoder`].

use std::io::{self, IoSlice, IoSliceMut};
use std::mem::MaybeUninit;
use std::os::fd::{BorrowedFd, OwnedFd};
use std::os::unix::net::UnixStream;

use rustix::io::Errno;
use rustix::net::{RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, ReturnFlags, SendAncillaryBuffer, SendAncillaryMessage, SendFlags};

/// The most descriptors one send or receive carries.
pub const MAX_FDS: usize = 8;

/// Room for control messages, aligned as cmsghdr so all 64 bytes are
/// usable: room for `MAX_FDS` descriptors, and no more than this
/// carries before a receive reports truncation.
#[repr(C, align(8))]
struct Control([MaybeUninit<u8>; 64]);

// The macro's room allows for an unaligned buffer; this one is aligned.
const _: () = assert!(rustix::cmsg_space!(ScmRights(MAX_FDS)) <= std::mem::size_of::<Control>() + std::mem::align_of::<Control>());

impl Control {
    fn new() -> Self { Self([MaybeUninit::uninit(); 64]) }
}

/// Retry CALL while a signal interrupts it.
fn restarting<T>(mut call: impl FnMut() -> rustix::io::Result<T>) -> io::Result<T> {
    loop {
        match call() {
            Err(Errno::INTR) => {}
            result => return result.map_err(io::Error::from),
        }
    }
}

/// Send some of BYTES, and FDS with their first byte. Returns how many
/// bytes went; the descriptors went if any byte did. On a nonblocking
/// socket this fails with `WouldBlock` instead of waiting.
///
/// # Errors
/// `InvalidInput` for descriptors with no byte or more than [`MAX_FDS`],
/// else the socket's own error.
pub fn send(socket: &UnixStream, bytes: &[u8], fds: &[BorrowedFd<'_>]) -> io::Result<usize> {
    if fds.len() > MAX_FDS || (bytes.is_empty() && !fds.is_empty()) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "descriptors need a byte to ride on, and at most eight go at once"));
    }
    let mut space = Control::new();
    let mut control = SendAncillaryBuffer::new(&mut space.0);
    if !fds.is_empty() && !control.push(SendAncillaryMessage::ScmRights(fds)) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "descriptors do not fit one control message"));
    }
    restarting(|| rustix::net::sendmsg(socket, &[IoSlice::new(bytes)], &mut control, SendFlags::NOSIGNAL))
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
    let mut space = Control::new();
    let mut control = RecvAncillaryBuffer::new(&mut space.0);
    let received = restarting(|| rustix::net::recvmsg(socket, &mut [IoSliceMut::new(buffer)], &mut control, RecvFlags::CMSG_CLOEXEC))?;
    // Each SCM_RIGHTS payload is descriptors the kernel just installed in
    // this process, owned here; credentials and anything else are let go.
    let arrived: Vec<OwnedFd> = control.drain().filter_map(|message| match message {
        RecvAncillaryMessage::ScmRights(rights) => Some(rights),
        _ => None,
    }).flatten().collect();
    // Some descriptors were discarded: which ones the records meant is lost.
    if received.flags.contains(ReturnFlags::CTRUNC) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "the peer sent more descriptors than one receive carries"));
    }
    fds.extend(arrived);
    Ok(received.bytes)
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
            assert!(rustix::io::fcntl_getfd(fd).unwrap().contains(rustix::io::FdFlags::CLOEXEC));
        }
    }

    /// Credentials beside the descriptors overflow the control buffer, as
    /// a peer sending too many descriptors would.
    #[test]
    fn truncated_descriptors_are_an_error_and_none_is_kept() {
        let (ours, theirs) = UnixStream::pair().unwrap();
        rustix::net::sockopt::set_socket_passcred(&theirs, true).unwrap();
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
