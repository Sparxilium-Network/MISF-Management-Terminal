//! Windows WebAuthn request adapter. Container/PRF crypto stays in LUKSbox.
//! API contract: https://github.com/microsoft/webauthn/blob/master/webauthn.h
use base64ct::{Base64UrlUnpadded, Encoding};
use luksbox_core::keyslot::FIDO2_CRED_ID_MAX;
use luksbox_fido2::{
    Credential, EnrollResult, Error, Fido2Authenticator, HmacSecret, HmacSecretRequest, RP_ID,
};
use std::{ptr, ptr::NonNull};
use windows::{
    Win32::{Networking::WindowsWebServices::*, UI::WindowsAndMessaging::GetForegroundWindow},
    core::{BOOL, HSTRING, PCWSTR},
};

type Result<T> = std::result::Result<T, Error>;

fn fail(message: &str) -> Error {
    Error::Other(message.into())
}

#[derive(Clone, Copy)]
struct Versions {
    make: u32,
    get: u32,
}
fn versions(api: u32) -> Result<Versions> {
    match api {
        0..=3 => Err(fail(
            "Windows WebAuthn API 4 or newer is required for PRF/hmac-secret",
        )),
        4..=5 => Ok(Versions { make: 5, get: 6 }),
        6 => Ok(Versions { make: 6, get: 6 }),
        _ => Ok(Versions { make: 7, get: 7 }),
    }
}

fn client_json(rp: &str, registration: bool) -> Result<Vec<u8>> {
    if rp != RP_ID {
        return Err(fail("Unexpected WebAuthn RP ID"));
    }
    let mut challenge = [0u8; 32];
    getrandom::fill(&mut challenge).map_err(|_| fail("Cannot generate WebAuthn challenge"))?;
    serde_json::to_vec(&serde_json::json!({
        "type": if registration { "webauthn.create" } else { "webauthn.get" },
        "challenge": Base64UrlUnpadded::encode_string(&challenge),
        "origin": format!("https://{rp}"),
        "crossOrigin": false,
    }))
    .map_err(|_| fail("Cannot encode WebAuthn client JSON"))
}

fn client_data(bytes: &mut [u8]) -> WEBAUTHN_CLIENT_DATA {
    WEBAUTHN_CLIENT_DATA {
        dwVersion: WEBAUTHN_CLIENT_DATA_CURRENT_VERSION,
        cbClientDataJSON: bytes.len() as u32,
        pbClientDataJSON: bytes.as_mut_ptr(),
        pwszHashAlgId: WEBAUTHN_HASH_ALGORITHM_SHA_256,
    }
}

fn native_error(operation: &str, api: u32, version: u32, error: windows::core::Error) -> Error {
    let reason = match error.code().0 as u32 {
        0x80090027 | 0x80070057 => {
            "Windows rejected the request parameters (NTE_INVALID_PARAMETER/E_INVALIDARG)"
        }
        0x80090036 | 0x800704c7 => "The user cancelled the verification",
        0x80090029 => "The operation or PRF extension is not supported",
        _ => "Windows authenticator operation failed",
    };
    // No client JSON, credential IDs, salts, PINs or secret output in diagnostics.
    Error::Other(format!(
        "{operation}: {reason}; HRESULT 0x{:08X}, WebAuthn API {api}, options v{version}",
        error.code().0 as u32
    ))
}

fn make_options(version: u32, attachment: u32) -> WEBAUTHN_AUTHENTICATOR_MAKE_CREDENTIAL_OPTIONS {
    WEBAUTHN_AUTHENTICATOR_MAKE_CREDENTIAL_OPTIONS {
        dwVersion: version,
        dwTimeoutMilliseconds: 60_000,
        dwAuthenticatorAttachment: attachment,
        dwUserVerificationRequirement: WEBAUTHN_USER_VERIFICATION_REQUIREMENT_REQUIRED,
        dwAttestationConveyancePreference: WEBAUTHN_ATTESTATION_CONVEYANCE_PREFERENCE_NONE,
        // PRF is a first-class option starting at make-options v6.
        bEnablePrf: BOOL::from(version >= 6),
        ..Default::default()
    }
}

pub(crate) struct WindowsAuthenticator {
    attachment: u32,
    api: u32,
    versions: Versions,
}
impl WindowsAuthenticator {
    pub(crate) fn new(hello: bool) -> Result<Self> {
        // Capability query only; does not start authentication or enumerate credentials.
        let api = unsafe { WebAuthNGetApiVersionNumber() };
        Ok(Self {
            attachment: if hello {
                WEBAUTHN_AUTHENTICATOR_ATTACHMENT_PLATFORM
            } else {
                WEBAUTHN_AUTHENTICATOR_ATTACHMENT_CROSS_PLATFORM
            },
            api,
            versions: versions(api)?,
        })
    }
}

struct Attestation(NonNull<WEBAUTHN_CREDENTIAL_ATTESTATION>);
impl Drop for Attestation {
    fn drop(&mut self) {
        // SAFETY: this guard exclusively owns the pointer returned by WebAuthn.
        unsafe {
            WebAuthNFreeCredentialAttestation(Some(self.0.as_ptr()));
        }
    }
}
struct Assertion(NonNull<WEBAUTHN_ASSERTION>);
impl Drop for Assertion {
    fn drop(&mut self) {
        // SAFETY: this guard exclusively owns the pointer returned by WebAuthn.
        unsafe {
            WebAuthNFreeAssertion(self.0.as_ptr());
        }
    }
}

/// All pointer backing storage stays alive until `invoke` returns; no self-referential
/// structure is moved after its addresses have been taken.
fn with_assertion_request<T>(
    attachment: u32,
    version: u32,
    rp: &str,
    requests: &[HmacSecretRequest<'_>],
    invoke: impl FnOnce(
        &WEBAUTHN_CLIENT_DATA,
        &WEBAUTHN_AUTHENTICATOR_GET_ASSERTION_OPTIONS,
    ) -> Result<T>,
) -> Result<T> {
    if requests.is_empty() || requests.len() > 8 {
        return Err(fail("Expected between one and eight Passkey candidates"));
    }
    for request in requests {
        if request.cred_id.is_empty() || request.cred_id.len() > FIDO2_CRED_ID_MAX {
            return Err(fail("Invalid Passkey credential length"));
        }
        if !request.prehash_salt {
            return Err(fail(
                "Legacy raw-salt FIDO2 slots are not supported on Windows",
            ));
        }
    }
    let mut json = client_json(rp, false)?;
    let client = client_data(&mut json);
    let mut ids: Vec<_> = requests.iter().map(|r| r.cred_id.to_vec()).collect();
    let mut salt_bytes: Vec<_> = requests.iter().map(|r| *r.salt).collect();
    let mut salts: Vec<_> = salt_bytes
        .iter_mut()
        .map(|salt| WEBAUTHN_HMAC_SECRET_SALT {
            cbFirst: 32,
            pbFirst: salt.as_mut_ptr(),
            ..Default::default()
        })
        .collect();
    let mut credentials: Vec<_> = ids
        .iter_mut()
        .map(|id| WEBAUTHN_CREDENTIAL_EX {
            dwVersion: WEBAUTHN_CREDENTIAL_EX_CURRENT_VERSION,
            cbId: id.len() as u32,
            pbId: id.as_mut_ptr(),
            pwszCredentialType: WEBAUTHN_CREDENTIAL_TYPE_PUBLIC_KEY,
            dwTransports: 0,
        })
        .collect();
    let mut credential_pointers: Vec<_> = credentials.iter_mut().map(|c| c as *mut _).collect();
    let mut allow = WEBAUTHN_CREDENTIAL_LIST {
        cCredentials: credentials.len() as u32,
        ppCredentials: credential_pointers.as_mut_ptr(),
    };
    let mut bindings: Vec<_> = ids
        .iter_mut()
        .zip(salts.iter_mut())
        .map(|(id, salt)| WEBAUTHN_CRED_WITH_HMAC_SECRET_SALT {
            cbCredID: id.len() as u32,
            pbCredID: id.as_mut_ptr(),
            pHmacSecretSalt: salt as *mut _,
        })
        .collect();
    let mut values = if requests.len() == 1 {
        WEBAUTHN_HMAC_SECRET_SALT_VALUES {
            pGlobalHmacSalt: &mut salts[0],
            ..Default::default()
        }
    } else {
        WEBAUTHN_HMAC_SECRET_SALT_VALUES {
            cCredWithHmacSecretSaltList: bindings.len() as u32,
            pCredWithHmacSecretSaltList: bindings.as_mut_ptr(),
            ..Default::default()
        }
    };
    let options = WEBAUTHN_AUTHENTICATOR_GET_ASSERTION_OPTIONS {
        dwVersion: version,
        dwTimeoutMilliseconds: 60_000,
        dwAuthenticatorAttachment: attachment,
        dwUserVerificationRequirement: WEBAUTHN_USER_VERIFICATION_REQUIREMENT_REQUIRED,
        pAllowCredentialList: &mut allow,
        pHmacSecretSaltValues: &mut values,
        // Keep raw-salt flag OFF: Windows applies the WebAuthn PRF prefix hash,
        // matching existing LUKSbox V4 slots. Client JSON challenges do not change it.
        dwFlags: 0,
        ..Default::default()
    };
    invoke(&client, &options)
}

impl Fido2Authenticator for WindowsAuthenticator {
    fn enroll(&mut self, rp: &str, user: &[u8], _pin: Option<&str>) -> Result<EnrollResult> {
        if user.is_empty() || user.len() > 64 {
            return Err(fail("Invalid Passkey user handle length"));
        }
        let mut json = client_json(rp, true)?;
        let client = client_data(&mut json);
        let rp_id = HSTRING::from(rp);
        let name = HSTRING::from("MISF Management Terminal");
        let rp_info = WEBAUTHN_RP_ENTITY_INFORMATION {
            dwVersion: WEBAUTHN_RP_ENTITY_INFORMATION_CURRENT_VERSION,
            pwszId: PCWSTR(rp_id.as_ptr()),
            pwszName: PCWSTR(name.as_ptr()),
            ..Default::default()
        };
        let mut user_id = user.to_vec();
        let user_info = WEBAUTHN_USER_ENTITY_INFORMATION {
            dwVersion: WEBAUTHN_USER_ENTITY_INFORMATION_CURRENT_VERSION,
            cbId: user_id.len() as u32,
            pbId: user_id.as_mut_ptr(),
            pwszName: PCWSTR(name.as_ptr()),
            pwszDisplayName: PCWSTR(name.as_ptr()),
            ..Default::default()
        };
        let mut algorithm = WEBAUTHN_COSE_CREDENTIAL_PARAMETER {
            dwVersion: WEBAUTHN_COSE_CREDENTIAL_PARAMETER_CURRENT_VERSION,
            pwszCredentialType: WEBAUTHN_CREDENTIAL_TYPE_PUBLIC_KEY,
            lAlg: -7,
        };
        let algorithms = WEBAUTHN_COSE_CREDENTIAL_PARAMETERS {
            cCredentialParameters: 1,
            pCredentialParameters: &mut algorithm,
        };
        let mut enabled = BOOL(1);
        let mut extension = WEBAUTHN_EXTENSION {
            pwszExtensionIdentifier: WEBAUTHN_EXTENSIONS_IDENTIFIER_HMAC_SECRET,
            cbExtension: size_of::<BOOL>() as u32,
            pvExtension: (&mut enabled as *mut BOOL).cast(),
        };
        let mut options = make_options(self.versions.make, self.attachment);
        if self.versions.make < 6 {
            options.Extensions = WEBAUTHN_EXTENSIONS {
                cExtensions: 1,
                pExtensions: &mut extension,
            };
        }
        // SAFETY: all buffers and native structures above outlive the synchronous call.
        // User verification is always REQUIRED; Windows owns the PIN/biometric UI.
        let pointer = unsafe {
            WebAuthNAuthenticatorMakeCredential(
                GetForegroundWindow(),
                &rp_info,
                &user_info,
                &algorithms,
                &client,
                Some(&options),
            )
        }
        .map_err(|error| {
            native_error("Passkey registration", self.api, self.versions.make, error)
        })?;
        let owned = Attestation(
            NonNull::new(pointer).ok_or_else(|| fail("WebAuthn returned a null attestation"))?,
        );
        // SAFETY: the WebAuthn allocation is live until `owned` is dropped.
        let attestation = unsafe { owned.0.as_ref() };
        if attestation.pbCredentialId.is_null()
            || attestation.cbCredentialId == 0
            || attestation.cbCredentialId as usize > FIDO2_CRED_ID_MAX
        {
            return Err(fail(
                "Authenticator credential ID cannot fit in a LUKSbox keyslot",
            ));
        }
        // SAFETY: pointer/length are supplied by the trusted OS allocation and bounded above.
        let id = unsafe {
            std::slice::from_raw_parts(
                attestation.pbCredentialId,
                attestation.cbCredentialId as usize,
            )
        }
        .to_vec();
        Ok(EnrollResult {
            credential: Credential { id },
        })
    }

    fn hmac_secret(
        &mut self,
        rp: &str,
        credential: &[u8],
        salt: &[u8; 32],
        prehash: bool,
        pin: Option<&str>,
    ) -> Result<HmacSecret> {
        self.hmac_secret_multi(
            rp,
            &[HmacSecretRequest {
                cred_id: credential,
                salt,
                prehash_salt: prehash,
            }],
            pin,
        )
        .map(|(_, secret)| secret)
    }

    fn hmac_secret_multi(
        &mut self,
        rp: &str,
        requests: &[HmacSecretRequest<'_>],
        _pin: Option<&str>,
    ) -> Result<(usize, HmacSecret)> {
        let rp_id = HSTRING::from(rp);
        with_assertion_request(
            self.attachment,
            self.versions.get,
            rp,
            requests,
            |client, options| {
                // SAFETY: with_assertion_request keeps every pointer backing buffer live.
                let pointer = unsafe {
                    WebAuthNAuthenticatorGetAssertion(
                        GetForegroundWindow(),
                        PCWSTR(rp_id.as_ptr()),
                        client,
                        Some(options),
                    )
                }
                .map_err(|error| {
                    native_error("Passkey verification", self.api, self.versions.get, error)
                })?;
                let owned = Assertion(
                    NonNull::new(pointer)
                        .ok_or_else(|| fail("WebAuthn returned a null assertion"))?,
                );
                // SAFETY: the owned OS allocation is live for this whole scope.
                let assertion = unsafe { owned.0.as_ref() };
                if assertion.dwVersion < 3 || assertion.pHmacSecret.is_null() {
                    return Err(Error::NoHmacSecret);
                }
                let credential = &assertion.Credential;
                if credential.pbId.is_null()
                    || credential.cbId == 0
                    || credential.cbId as usize > FIDO2_CRED_ID_MAX
                {
                    return Err(fail("Invalid credential in WebAuthn assertion"));
                }
                // SAFETY: native output pointers come from the OS allocation; lengths bounded.
                let id = unsafe {
                    std::slice::from_raw_parts(credential.pbId, credential.cbId as usize)
                };
                let index = requests
                    .iter()
                    .position(|r| r.cred_id == id)
                    .ok_or_else(|| {
                        fail("WebAuthn selected a credential outside the requested keyslots")
                    })?;
                // SAFETY: pHmacSecret is available since assertion v3, checked above.
                let output = unsafe { &*assertion.pHmacSecret };
                if output.cbFirst != 32 || output.pbFirst.is_null() {
                    return Err(Error::NoHmacSecret);
                }
                let mut secret = zeroize::Zeroizing::new([0u8; 32]);
                // SAFETY: OS owns a live cbFirst-byte output, validated to exactly 32 bytes.
                unsafe {
                    ptr::copy_nonoverlapping(output.pbFirst, secret.as_mut_ptr(), 32);
                }
                Ok((index, HmacSecret(*secret)))
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_ceremony_uses_fresh_valid_client_json() {
        let a = client_json(RP_ID, false).unwrap();
        let b = client_json(RP_ID, false).unwrap();
        let a: serde_json::Value = serde_json::from_slice(&a).unwrap();
        let b: serde_json::Value = serde_json::from_slice(&b).unwrap();
        assert_eq!(a["type"], "webauthn.get");
        assert_eq!(a["origin"], format!("https://{RP_ID}"));
        assert_eq!(a["crossOrigin"], false);
        let challenge = Base64UrlUnpadded::decode_vec(a["challenge"].as_str().unwrap()).unwrap();
        assert_eq!(challenge.len(), 32);
        assert_ne!(a["challenge"], b["challenge"]);
        let create: serde_json::Value =
            serde_json::from_slice(&client_json(RP_ID, true).unwrap()).unwrap();
        assert_eq!(create["type"], "webauthn.create");
    }

    #[test]
    fn options_versions_match_os_and_uv_is_required() {
        assert!(versions(3).is_err());
        for (api, make, get) in [(4, 5, 6), (5, 5, 6), (6, 6, 6), (7, 7, 7), (9, 7, 7)] {
            let selected = versions(api).unwrap();
            assert_eq!((selected.make, selected.get), (make, get));
            let options = make_options(make, WEBAUTHN_AUTHENTICATOR_ATTACHMENT_PLATFORM);
            assert_eq!(
                options.dwUserVerificationRequirement,
                WEBAUTHN_USER_VERIFICATION_REQUIREMENT_REQUIRED
            );
            assert_eq!(options.bEnablePrf.as_bool(), make >= 6);
            assert!(!options.bRequireResidentKey.as_bool());
        }
    }

    #[test]
    fn native_request_keeps_salts_and_credentials_paired_without_raw_salt_flag() {
        for count in [1usize, 2, 8] {
            let ids: Vec<_> = (0..count).map(|i| vec![i as u8; 32]).collect();
            let salts: Vec<_> = (0..count).map(|i| [i as u8 + 1; 32]).collect();
            let requests: Vec<_> = ids
                .iter()
                .zip(&salts)
                .map(|(id, salt)| HmacSecretRequest {
                    cred_id: id,
                    salt,
                    prehash_salt: true,
                })
                .collect();
            with_assertion_request(
                WEBAUTHN_AUTHENTICATOR_ATTACHMENT_CROSS_PLATFORM,
                7,
                RP_ID,
                &requests,
                |client, options| {
                    // SAFETY: this test callback runs while the request backing storage is live.
                    unsafe {
                        let json = std::slice::from_raw_parts(
                            client.pbClientDataJSON,
                            client.cbClientDataJSON as usize,
                        );
                        assert_eq!(
                            serde_json::from_slice::<serde_json::Value>(json).unwrap()["type"],
                            "webauthn.get"
                        );
                        assert_eq!(
                            options.dwUserVerificationRequirement,
                            WEBAUTHN_USER_VERIFICATION_REQUIREMENT_REQUIRED
                        );
                        assert_eq!(
                            options.dwFlags & WEBAUTHN_AUTHENTICATOR_HMAC_SECRET_VALUES_FLAG,
                            0
                        );
                        assert_eq!((*options.pAllowCredentialList).cCredentials as usize, count);
                        let values = &*options.pHmacSecretSaltValues;
                        if count == 1 {
                            assert!(!values.pGlobalHmacSalt.is_null());
                            assert_eq!(values.cCredWithHmacSecretSaltList, 0);
                        } else {
                            assert!(values.pGlobalHmacSalt.is_null());
                            let bindings = std::slice::from_raw_parts(
                                values.pCredWithHmacSecretSaltList,
                                count,
                            );
                            for (index, binding) in bindings.iter().enumerate() {
                                assert_eq!(
                                    std::slice::from_raw_parts(
                                        binding.pbCredID,
                                        binding.cbCredID as usize
                                    ),
                                    ids[index]
                                );
                                assert_eq!(
                                    std::slice::from_raw_parts(
                                        (*binding.pHmacSecretSalt).pbFirst,
                                        32
                                    ),
                                    salts[index]
                                );
                            }
                        }
                    }
                    Ok(())
                },
            )
            .unwrap();
        }
    }

    #[test]
    fn parameter_errors_are_not_reported_as_user_cancellation() {
        let error = native_error(
            "Passkey verification",
            9,
            7,
            windows::core::Error::from_hresult(windows::core::HRESULT(0x80090027u32 as i32)),
        )
        .to_string();
        assert!(error.contains("NTE_INVALID_PARAMETER"));
        assert!(!error.contains("cancelled"));
        assert!(error.contains("WebAuthn API 9"));
    }
}
