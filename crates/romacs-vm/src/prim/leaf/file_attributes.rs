use super::*;

impl Leaf<'_> {
    pub(crate) fn file_system_bytes(&self, path: &str) -> std::io::Result<[i128; 3]> {
        let path = std::ffi::CString::new(path).map_err(|_| std::io::Error::from_raw_os_error(libc::EINVAL))?;
        let mut usage = std::mem::MaybeUninit::<libc::statvfs>::uninit();
        if unsafe { libc::statvfs(path.as_ptr(), usage.as_mut_ptr()) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        let usage = unsafe { usage.assume_init() };
        let block = i128::from(usage.f_frsize);
        Ok([i128::from(usage.f_blocks) * block, i128::from(usage.f_bfree) * block,
            i128::from(usage.f_bavail as i64) * block])
    }

    pub(crate) fn file_attribute_identity(&mut self, id: u32, group: bool, named: bool) -> Value {
        if named {
            let mut buffer = vec![0u8; 1024];
            loop {
                let (status, name) = if group {
                    let mut entry = std::mem::MaybeUninit::<libc::group>::uninit();
                    let mut result = std::ptr::null_mut();
                    let status = unsafe { libc::getgrgid_r(id, entry.as_mut_ptr(), buffer.as_mut_ptr().cast(), buffer.len(), &mut result) };
                    (status, if status == 0 && !result.is_null() { unsafe { (*result).gr_name } } else { std::ptr::null_mut() })
                } else {
                    let mut entry = std::mem::MaybeUninit::<libc::passwd>::uninit();
                    let mut result = std::ptr::null_mut();
                    let status = unsafe { libc::getpwuid_r(id, entry.as_mut_ptr(), buffer.as_mut_ptr().cast(), buffer.len(), &mut result) };
                    (status, if status == 0 && !result.is_null() { unsafe { (*result).pw_name } } else { std::ptr::null_mut() })
                };
                if status == libc::ERANGE && buffer.len() < 1_048_576 {
                    buffer.resize(buffer.len() * 2, 0);
                    continue;
                }
                if !name.is_null() {
                    let bytes = unsafe { std::ffi::CStr::from_ptr(name) }.to_bytes();
                    if std::str::from_utf8(bytes).is_ok() {
                        return self.make_string(bytes.to_vec(), !bytes.is_ascii());
                    }
                }
                break;
            }
        }
        self.bigint(id.into())
    }
}
