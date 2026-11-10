#[cfg(windows)]
pub fn certificate() -> bricks_control::Result<Option<String>> {
    use windows_sys::Win32::UI::Controls::Dialogs::*;
    let mut filename = [0u16; 32768];
    let filter: Vec<u16> = "Public Bricks certificate (*.der)\0*.der\0All files\0*.*\0\0"
        .encode_utf16()
        .collect();
    let title: Vec<u16> = "Choose the other PC's PUBLIC certificate\0"
        .encode_utf16()
        .collect();
    let mut dialog = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        lpstrFile: filename.as_mut_ptr(),
        nMaxFile: filename.len() as u32,
        lpstrFilter: filter.as_ptr(),
        lpstrTitle: title.as_ptr(),
        Flags: OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR,
        ..Default::default()
    };
    // SAFETY: Mutable UTF-16 output and immutable double-NUL filter/title remain
    // alive throughout the synchronous native dialog. No active session uses it.
    unsafe {
        if GetOpenFileNameW(&mut dialog) == 0 {
            let error = CommDlgExtendedError();
            if error == 0 {
                return Ok(None);
            }
            return Err(format!("Could not choose certificate ({error}).").into());
        }
    }
    let end = filename
        .iter()
        .position(|&value| value == 0)
        .unwrap_or(filename.len());
    Ok(Some(String::from_utf16(&filename[..end])?))
}
#[cfg(not(windows))]
pub fn certificate() -> bricks_control::Result<Option<String>> {
    Err("Certificate picker currently requires Windows.".into())
}
