//! Secret storage.
//!
//! On Windows, values are stored in the Windows Credential Manager under the
//! "UniversalMCPManager" service. Nothing is ever written to logs or to the
//! SQLite database: the database only keeps metadata (`secret_refs`) so the UI
//! can list which slots exist.
//!
//! On other platforms the store reports itself as unavailable and the UI will
//! require literal values instead — no insecure fallback file is created.

use crate::errors::{AppError, AppResult};

const SERVICE: &str = "UniversalMCPManager";

#[derive(Clone)]
pub struct SecretStore {
    service: String,
}

impl Default for SecretStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SecretStore {
    pub fn new() -> Self {
        Self { service: SERVICE.to_string() }
    }

    pub fn is_available(&self) -> bool {
        cfg!(target_os = "windows")
    }

    pub fn account(&self, mcp_id: &str, slot: &str, key: &str) -> String {
        format!("{}|{}|{}|{}", self.service, mcp_id, slot, key)
    }

    pub fn set(&self, mcp_id: &str, slot: &str, key: &str, value: &str) -> AppResult<()> {
        platform::set(&self.account(mcp_id, slot, key), value)
    }

    pub fn get(&self, mcp_id: &str, slot: &str, key: &str) -> AppResult<Option<String>> {
        platform::get(&self.account(mcp_id, slot, key))
    }

    pub fn delete(&self, mcp_id: &str, slot: &str, key: &str) -> AppResult<()> {
        platform::delete(&self.account(mcp_id, slot, key))
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use windows_sys::Win32::Foundation::ERROR_NOT_FOUND;
    use windows_sys::Win32::Security::Credentials::{
        CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE,
        CRED_TYPE_GENERIC,
    };

    use super::AppError;
    use crate::errors::AppResult;

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub fn set(target: &str, secret: &str) -> AppResult<()> {
        let mut target_wide = wide(target);
        let mut blob = secret.as_bytes().to_vec();
        let credential = CREDENTIALW {
            Flags: 0,
            Type: CRED_TYPE_GENERIC,
            TargetName: target_wide.as_mut_ptr(),
            Comment: std::ptr::null_mut(),
            LastWritten: unsafe { std::mem::zeroed() },
            CredentialBlobSize: blob.len() as u32,
            CredentialBlob: blob.as_mut_ptr(),
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            AttributeCount: 0,
            Attributes: std::ptr::null_mut(),
            TargetAlias: std::ptr::null_mut(),
            UserName: std::ptr::null_mut(),
        };
        let ok = unsafe { CredWriteW(&credential, 0) };
        if ok == 0 {
            let code = std::io::Error::last_os_error();
            return Err(AppError::secret_store(format!(
                "Windows Credential Manager rejected the value: {code}"
            )));
        }
        Ok(())
    }

    pub fn get(target: &str) -> AppResult<Option<String>> {
        let target_wide = wide(target);
        let mut credential: *mut CREDENTIALW = std::ptr::null_mut();
        let ok = unsafe { CredReadW(target_wide.as_ptr(), CRED_TYPE_GENERIC, 0, &mut credential) };
        if ok == 0 {
            let error = std::io::Error::last_os_error();
            let not_found = error.raw_os_error() == Some(ERROR_NOT_FOUND as i32);
            if not_found {
                return Ok(None);
            }
            return Err(AppError::secret_store(format!(
                "Windows Credential Manager read failed: {error}"
            )));
        }
        let result = unsafe {
            let cred = &*credential;
            if cred.CredentialBlobSize == 0 || cred.CredentialBlob.is_null() {
                String::new()
            } else {
                let slice =
                    std::slice::from_raw_parts(cred.CredentialBlob, cred.CredentialBlobSize as usize);
                String::from_utf8_lossy(slice).to_string()
            }
        };
        unsafe { CredFree(credential as *mut std::ffi::c_void) };
        Ok(Some(result))
    }

    pub fn delete(target: &str) -> AppResult<()> {
        let target_wide = wide(target);
        let ok = unsafe { CredDeleteW(target_wide.as_ptr(), CRED_TYPE_GENERIC, 0) };
        if ok == 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_NOT_FOUND as i32) {
                return Ok(());
            }
            return Err(AppError::secret_store(format!(
                "Windows Credential Manager delete failed: {error}"
            )));
        }
        Ok(())
    }
}

#[cfg(not(target_os = "windows"))]
mod platform {
    use super::AppError;
    use crate::errors::AppResult;

    pub fn set(_target: &str, _secret: &str) -> AppResult<()> {
        Err(AppError::secret_store(
            "OS credential store integration is only implemented for Windows in this build",
        ))
    }

    pub fn get(_target: &str) -> AppResult<Option<String>> {
        Ok(None)
    }

    pub fn delete(_target: &str) -> AppResult<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_names_are_namespaced() {
        let store = SecretStore::new();
        let account = store.account("mcp-1", "env", "GITHUB_TOKEN");
        assert!(account.starts_with(SERVICE));
        assert!(account.contains("mcp-1"));
        assert!(account.contains("GITHUB_TOKEN"));
    }
}
