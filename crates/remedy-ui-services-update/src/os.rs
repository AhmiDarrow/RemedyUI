//! Authenticode on Windows: whether a file's embedded signature holds, and which certificate
//! signed it. The only unsafe code in the updater.

/// What Windows says about a file's signature.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SignatureStatus {
    /// Signed, and the chain is trusted.
    Valid,
    /// Not signed.
    NotSigned,
    /// Signed, but the file changed since (never acceptable).
    HashMismatch,
    /// Signed by a certificate Windows does not trust (a self-signed publisher).
    NotTrusted,
    /// Anything else, with Windows' code.
    Other(i32),
}

/// A file's Authenticode signature: its status and the signer certificate's SHA-1 thumbprint
/// (uppercase hex) when it has one.
#[cfg(windows)]
pub fn authenticode(path: &std::path::Path) -> (SignatureStatus, Option<String>) {
    use std::os::windows::ffi::OsStrExt;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    (trust(&wide), thumbprint(&wide))
}

#[cfg(windows)]
fn trust(wide: &[u16]) -> SignatureStatus {
    use windows_sys::Win32::Security::WinTrust::{
        WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA, WINTRUST_DATA_0, WINTRUST_FILE_INFO,
        WTD_CACHE_ONLY_URL_RETRIEVAL, WTD_CHOICE_FILE, WTD_REVOKE_NONE, WTD_STATEACTION_CLOSE,
        WTD_STATEACTION_VERIFY, WTD_UI_NONE, WinVerifyTrust,
    };
    let mut file = WINTRUST_FILE_INFO {
        cbStruct: std::mem::size_of::<WINTRUST_FILE_INFO>() as u32,
        pcwszFilePath: wide.as_ptr(),
        hFile: std::ptr::null_mut(),
        pgKnownSubject: std::ptr::null_mut(),
    };
    let mut data = WINTRUST_DATA {
        cbStruct: std::mem::size_of::<WINTRUST_DATA>() as u32,
        dwUIChoice: WTD_UI_NONE,
        fdwRevocationChecks: WTD_REVOKE_NONE,
        dwUnionChoice: WTD_CHOICE_FILE,
        Anonymous: WINTRUST_DATA_0 { pFile: &mut file },
        dwStateAction: WTD_STATEACTION_VERIFY,
        dwProvFlags: WTD_CACHE_ONLY_URL_RETRIEVAL,
        ..Default::default()
    };
    let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
    // SAFETY: `data` and `file` point at live locals (and a NUL-terminated path) for both calls;
    // the second call releases what the first one allocated.
    let code = unsafe {
        let c = WinVerifyTrust(std::ptr::null_mut(), &mut action, (&raw mut data).cast());
        data.dwStateAction = WTD_STATEACTION_CLOSE;
        let _ = WinVerifyTrust(std::ptr::null_mut(), &mut action, (&raw mut data).cast());
        c
    };
    // Status codes are HRESULTs; compare as unsigned.
    match code as u32 {
        0 => SignatureStatus::Valid,
        0x800B_0100 => SignatureStatus::NotSigned,
        0x8009_6010 => SignatureStatus::HashMismatch,
        0x800B_0004 | 0x800B_0109 | 0x800B_010A | 0x800B_0101 | 0x800B_0105 | 0x800B_0110 => {
            SignatureStatus::NotTrusted
        }
        _ => SignatureStatus::Other(code),
    }
}

#[cfg(windows)]
fn thumbprint(wide: &[u16]) -> Option<String> {
    use windows_sys::Win32::Security::Cryptography::{
        CERT_FIND_SUBJECT_CERT, CERT_HASH_PROP_ID, CERT_INFO, CERT_QUERY_CONTENT_FLAG_PKCS7_SIGNED_EMBED,
        CERT_QUERY_FORMAT_FLAG_BINARY, CERT_QUERY_OBJECT_FILE, CMSG_SIGNER_INFO, CMSG_SIGNER_INFO_PARAM,
        CertCloseStore, CertFindCertificateInStore, CertFreeCertificateContext,
        CertGetCertificateContextProperty, CryptMsgClose, CryptMsgGetParam, CryptQueryObject, HCERTSTORE,
        PKCS_7_ASN_ENCODING, X509_ASN_ENCODING,
    };
    let mut encoding = 0u32;
    let mut content = 0u32;
    let mut format = 0u32;
    let mut store: HCERTSTORE = std::ptr::null_mut();
    let mut msg: *mut core::ffi::c_void = std::ptr::null_mut();
    // SAFETY: every out pointer is a live local; the store and message are closed below.
    let ok = unsafe {
        CryptQueryObject(
            CERT_QUERY_OBJECT_FILE,
            wide.as_ptr().cast(),
            CERT_QUERY_CONTENT_FLAG_PKCS7_SIGNED_EMBED,
            CERT_QUERY_FORMAT_FLAG_BINARY,
            0,
            &mut encoding,
            &mut content,
            &mut format,
            &mut store,
            &mut msg,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return None;
    }
    let result = (|| {
        let mut size = 0u32;
        // SAFETY: a size query, then a read into a buffer of that size (u64-aligned for the struct).
        unsafe {
            if CryptMsgGetParam(msg, CMSG_SIGNER_INFO_PARAM, 0, std::ptr::null_mut(), &mut size) == 0
                || size == 0
            {
                return None;
            }
        }
        let words = (size as usize).div_ceil(8);
        let mut buf = vec![0u64; words];
        // SAFETY: `buf` holds `size` bytes, aligned for CMSG_SIGNER_INFO.
        let info = unsafe {
            if CryptMsgGetParam(msg, CMSG_SIGNER_INFO_PARAM, 0, buf.as_mut_ptr().cast(), &mut size) == 0 {
                return None;
            }
            &*(buf.as_ptr().cast::<CMSG_SIGNER_INFO>())
        };
        let find = CERT_INFO { Issuer: info.Issuer, SerialNumber: info.SerialNumber, ..Default::default() };
        // SAFETY: `find` borrows `info`, which `buf` keeps alive; the context is freed below.
        let ctx = unsafe {
            CertFindCertificateInStore(
                store,
                X509_ASN_ENCODING | PKCS_7_ASN_ENCODING,
                0,
                CERT_FIND_SUBJECT_CERT,
                (&raw const find).cast(),
                std::ptr::null(),
            )
        };
        if ctx.is_null() {
            return None;
        }
        let mut hash = [0u8; 20];
        let mut len = hash.len() as u32;
        // SAFETY: `hash` has `len` bytes; the context came from the store above.
        let got = unsafe {
            let r =
                CertGetCertificateContextProperty(ctx, CERT_HASH_PROP_ID, hash.as_mut_ptr().cast(), &mut len);
            CertFreeCertificateContext(ctx);
            r
        };
        (got != 0).then(|| {
            hash.get(..len as usize).unwrap_or_default().iter().map(|b| format!("{b:02X}")).collect()
        })
    })();
    // SAFETY: both came from CryptQueryObject above.
    unsafe {
        CryptMsgClose(msg);
        CertCloseStore(store, 0);
    }
    result
}

/// Authenticode is Windows' signature: elsewhere there is nothing to read.
#[cfg(not(windows))]
pub fn authenticode(_path: &std::path::Path) -> (SignatureStatus, Option<String>) {
    (SignatureStatus::NotSigned, None)
}
