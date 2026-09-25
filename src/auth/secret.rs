use anyhow::{Result, ensure};
use std::io::Read;
use zeroize::Zeroizing;

pub const MAX_KEY_BYTES: usize = 4096;

// Intentionally no Debug/Display/Serialize implementation.
pub struct Secret(Zeroizing<String>);
impl Secret {
    pub fn parse(value: String) -> Result<Self> {
        let value = Zeroizing::new(value);
        let trimmed = value.trim();
        ensure!(
            !trimmed.is_empty()
                && trimmed.len() <= MAX_KEY_BYTES
                && trimmed.bytes().all(|b| b.is_ascii_graphic()),
            "Invalid API key: provide one nonempty key without spaces or embedded newlines"
        );
        Ok(Self(Zeroizing::new(trimmed.to_owned())))
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
}

pub fn read_stdin(input: impl Read) -> Result<Secret> {
    let mut value = Zeroizing::new(String::new());
    input
        .take((MAX_KEY_BYTES + 1) as u64)
        .read_to_string(&mut value)
        .map_err(|_| anyhow::anyhow!("Could not read a UTF-8 API key from stdin"))?;
    ensure!(value.len() <= MAX_KEY_BYTES, "API key input is too long");
    Secret::parse(std::mem::take(&mut *value))
}
