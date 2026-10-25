use crate::Vm;

pub(super) fn snapshot() -> Vec<Vec<u8>> {
    std::env::vars_os().map(|(name, value)| {
        let mut entry = name.into_encoded_bytes();
        entry.push(b'=');
        entry.extend(value.into_encoded_bytes());
        entry
    }).collect()
}

pub(crate) fn multibyte(bytes: &[u8]) -> bool {
    if bytes.is_ascii() { return false; }
    let mut remaining = bytes;
    while !remaining.is_empty() {
        let Some((code, width)) = crate::lstring::decode_char(remaining) else { return false; };
        if code > crate::lstring::MAX_5_BYTE_CHAR { return false; }
        remaining = &remaining[width..];
    }
    true
}

impl Vm {
    pub(super) fn initialize_environment_slot(&mut self, name: &str) {
        if !matches!(name, "process-environment" | "initial-environment") { return; }
        let Some(entries) = self.environment_snapshot.take() else { return; };
        let values = entries.into_iter().rev().map(|bytes| {
            let multibyte = multibyte(&bytes);
            self.make_lisp_string(bytes, multibyte)
        }).collect::<Vec<_>>();
        let process = self.list_value(&values);
        let initial = self.list_value(&values);
        self.declare_special_raw("process-environment", process);
        self.declare_special_raw("initial-environment", initial);
    }
}
