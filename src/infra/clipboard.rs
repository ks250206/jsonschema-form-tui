use anyhow::{Context, Result};

pub fn set_text(text: &str) -> Result<()> {
    let mut clipboard = arboard::Clipboard::new().context("failed to open clipboard")?;
    clipboard
        .set_text(text.to_owned())
        .context("failed to write clipboard")
}
