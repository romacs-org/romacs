use super::*;

impl Leaf<'_> {
    pub(crate) fn file_system_bytes(&self, path: &str) -> std::io::Result<[i128; 3]> {
        let path = std::ffi::CString::new(path).map_err(|_| std::io::Error::from_raw_os_error(libc::EINVAL))?;
        let usage = crate::host::statvfs(&path)?;
        let block = i128::from(usage.fragment);
        Ok([i128::from(usage.blocks) * block, i128::from(usage.free) * block, i128::from(usage.available as i64) * block])
    }

    pub(crate) fn file_attribute_identity(&mut self, id: u32, group: bool, named: bool) -> Value {
        if named {
            let mut buffer = 1024;
            loop {
                let name = if group { crate::host::group_by_gid(id, buffer) }
                    else { crate::host::user_by_uid(id, buffer).and_then(|entry| entry.name.ok_or(0)) };
                match name {
                    Err(libc::ERANGE) if buffer < 1_048_576 => buffer *= 2,
                    Ok(bytes) if std::str::from_utf8(&bytes).is_ok() => {
                        let multibyte = !bytes.is_ascii();
                        return self.make_string(bytes, multibyte);
                    }
                    _ => break,
                }
            }
        }
        self.bigint(id.into())
    }
}
