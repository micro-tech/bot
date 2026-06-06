// Task 78: Proper group/permission handling
use std::fs;
use std::os::unix::fs::PermissionsExt;

pub fn set_socket_permissions(path: &str, mode: u32) -> anyhow::Result<()> {
    let mut perms = fs::metadata(path)?.permissions();
    perms.set_mode(mode);
    fs::set_permissions(path, perms)?;
    Ok(())
}
