//! Handle-based checks for local NTFS objects. Handles share delete access so
//! sampling cannot keep a user-owned socket alive or prevent its replacement.
use crate::Result;
use std::os::windows::io::{FromRawHandle, OwnedHandle};
use std::{
    fs::{File, OpenOptions},
    mem::{size_of, zeroed},
    os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
    path::{Path, PathBuf},
    ptr::{addr_of_mut, null_mut},
};
use windows_sys::Win32::{
    Foundation::*,
    Security::{Authorization::*, *},
    Storage::FileSystem::*,
    System::{JobObjects::*, Threading::*},
};

fn owned(h: HANDLE) -> Result<OwnedHandle> {
    if h.is_null() || h == INVALID_HANDLE_VALUE {
        Err("access_denied")
    } else {
        Ok(unsafe { OwnedHandle::from_raw_handle(h) })
    }
}
fn open(path: &Path) -> Result<File> {
    use std::path::{Component, Prefix};
    if !path.is_absolute()
        || !matches!(path.components().next(),Some(Component::Prefix(p)) if matches!(p.kind(),Prefix::Disk(_)))
    {
        return Err("local_path_required");
    }
    OpenOptions::new()
        .access_mode(0x0002_0000 | FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|_| "unavailable")
}
struct Descriptor(PSECURITY_DESCRIPTOR);
impl Drop for Descriptor {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.0);
        }
    }
}
fn private(file: &File) -> Result<()> {
    unsafe {
        let mut token = null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err("acl_unavailable");
        }
        let token = owned(token)?;
        let mut bytes = 0;
        GetTokenInformation(token.as_raw_handle(), TokenUser, null_mut(), 0, &mut bytes);
        if bytes == 0 || bytes > 65536 {
            return Err("acl_unavailable");
        }
        let mut buf = vec![0usize; (bytes as usize).div_ceil(size_of::<usize>())];
        if GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            buf.as_mut_ptr().cast(),
            bytes,
            &mut bytes,
        ) == 0
        {
            return Err("acl_unavailable");
        }
        let user = (*(buf.as_ptr() as *const TOKEN_USER)).User.Sid;
        let (mut owner, mut acl, mut sd) = (null_mut(), null_mut(), null_mut());
        if GetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            null_mut(),
            &mut acl,
            null_mut(),
            &mut sd,
        ) != 0
        {
            return Err("acl_unavailable");
        }
        let _sd = Descriptor(sd);
        if owner.is_null() || EqualSid(owner, user) == 0 || acl.is_null() {
            return Err("acl_not_private");
        }
        for i in 0..(*acl).AceCount {
            let mut raw = null_mut();
            if GetAce(acl, u32::from(i), &mut raw) == 0 {
                return Err("acl_unavailable");
            }
            match (*(raw as *const ACE_HEADER)).AceType {
                1 => continue, // ACCESS_DENIED_ACE
                0 => {
                    let sid = addr_of_mut!((*(raw as *mut ACCESS_ALLOWED_ACE)).SidStart).cast();
                    if EqualSid(sid, user) == 0
                        && IsWellKnownSid(sid, WinLocalSystemSid) == 0
                        && IsWellKnownSid(sid, WinBuiltinAdministratorsSid) == 0
                    {
                        return Err("acl_not_private");
                    }
                }
                _ => return Err("acl_not_private"),
            }
        }
        Ok(())
    }
}
pub fn directory(path: &Path) -> Result<()> {
    let file = open(path)?;
    private(&file)?;
    let info = attributes(&file)?;
    if info.FileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0
        || info.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
    {
        return Err("invalid_directory");
    }
    Ok(())
}
fn attributes(f: &File) -> Result<FILE_ATTRIBUTE_TAG_INFO> {
    unsafe {
        let mut a = zeroed();
        if GetFileInformationByHandleEx(
            f.as_raw_handle(),
            FileAttributeTagInfo,
            &mut a as *mut _ as _,
            size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
        ) == 0
        {
            return Err("file_identity");
        }
        Ok(a)
    }
}
fn identity(f: &File) -> Result<String> {
    unsafe {
        let mut a: BY_HANDLE_FILE_INFORMATION = zeroed();
        if GetFileInformationByHandle(f.as_raw_handle(), &mut a) == 0 {
            return Err("file_identity");
        }
        Ok(format!(
            "{:x}-{:x}{:08x}-{:x}{:08x}",
            a.dwVolumeSerialNumber,
            a.nFileIndexHigh,
            a.nFileIndexLow,
            a.ftCreationTime.dwHighDateTime,
            a.ftCreationTime.dwLowDateTime
        ))
    }
}
pub fn socket(path: &Path) -> Result<String> {
    directory(path.parent().ok_or("invalid_directory")?)?;
    let file = open(path)?;
    private(&file)?;
    if attributes(&file)?.ReparseTag != 0x80000023 {
        return Err("not_unix_socket");
    }
    identity(&file)
}
pub fn executable(path: &Path) -> Result<String> {
    identity(&open(path)?)
}
pub fn process_binary(pid: u32) -> Result<String> {
    unsafe {
        let h = owned(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid))?;
        let mut buf = [0u16; 32768];
        let mut n = buf.len() as u32;
        if QueryFullProcessImageNameW(h.as_raw_handle(), 0, buf.as_mut_ptr(), &mut n) == 0 {
            return Err("process_identity");
        }
        use std::os::windows::ffi::OsStringExt;
        executable(&PathBuf::from(std::ffi::OsString::from_wide(
            &buf[..n as usize],
        )))
    }
}
pub fn job(child: &std::process::Child) -> Result<OwnedHandle> {
    unsafe {
        let h = owned(CreateJobObjectW(std::ptr::null(), std::ptr::null()))?;
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if SetInformationJobObject(
            h.as_raw_handle(),
            JobObjectExtendedLimitInformation,
            &limits as *const _ as _,
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        ) == 0
            || AssignProcessToJobObject(h.as_raw_handle(), child.as_raw_handle()) == 0
        {
            return Err("worker_isolation");
        }
        Ok(h)
    }
}

pub fn private_file(file: &File) -> Result<()> {
    private(file)
}
