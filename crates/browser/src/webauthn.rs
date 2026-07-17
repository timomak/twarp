//! Software WebAuthn (passkey) authenticator for the browser pane.
//!
//! WKWebView only runs real WebAuthn ceremonies for apps holding Apple's
//! restricted `com.apple.developer.web-browser.public-key-credential`
//! entitlement (which needs an Apple-approved provisioning profile — signing
//! without one makes launchd refuse to spawn the app). Instead, the injected
//! page script overrides `navigator.credentials.create/get` and bridges
//! requests here, where twarp itself acts as the authenticator: P-256 keys
//! stored in the macOS Keychain, user verification via Touch ID (done on the
//! native side before the request reaches us). Passkeys created here live in
//! twarp only — existing iCloud Keychain passkeys are not visible.
//!
//! Follows the same authenticator-data layout iCloud Keychain uses: ES256
//! (COSE alg -7), `fmt: "none"` attestation, a constant zero signature
//! counter, and an all-zero AAGUID.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use p256::ecdsa::signature::Signer;
use p256::ecdsa::{Signature, SigningKey};
use p256::elliptic_curve::sec1::ToEncodedPoint;
use p256::SecretKey;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const KEYCHAIN_SERVICE: &str = "twarp-browser-webauthn";
const KEYCHAIN_ACCOUNT: &str = "credentials";
/// ES256 — the only algorithm this authenticator implements (like iCloud
/// Keychain passkeys).
const COSE_ALG_ES256: i64 = -7;
/// DER SubjectPublicKeyInfo header for an uncompressed P-256 point.
const P256_SPKI_PREFIX: &[u8] = &[
    0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a,
    0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
];

const FLAG_USER_PRESENT: u8 = 0x01;
const FLAG_USER_VERIFIED: u8 = 0x04;
const FLAG_ATTESTED_CREDENTIAL_DATA: u8 = 0x40;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BridgeRequest {
    // Echoed back to the page from the raw JSON (see `deliver`), not via this
    // field — kept here so the request shape stays self-documenting.
    #[allow(dead_code)]
    id: String,
    kind: String,
    origin: String,
    rp_id: Option<String>,
    challenge: Option<String>,
    user_id: Option<String>,
    user_name: Option<String>,
    algs: Option<Vec<i64>>,
    exclude_ids: Option<Vec<String>>,
    allow_ids: Option<Vec<String>>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct StoredCredential {
    cred_id: String,
    rp_id: String,
    user_handle: String,
    user_name: String,
    private_key: String,
}

#[derive(Debug)]
struct WebAuthnError {
    name: &'static str,
    message: String,
}

impl WebAuthnError {
    fn not_allowed(message: impl Into<String>) -> Self {
        Self {
            name: "NotAllowedError",
            message: message.into(),
        }
    }
}

type WebAuthnResult<T> = std::result::Result<T, WebAuthnError>;

/// Registers the bridge handler. Idempotent; called when the first browser
/// engine is created (main thread).
pub(crate) fn install() {
    #[cfg(target_os = "macos")]
    {
        use std::sync::Once;
        static INSTALL: Once = Once::new();
        INSTALL.call_once(|| {
            crate::MacWindow::set_browser_webauthn_handler(Box::new(
                |window_id, webview_id, request_json, user_verified| {
                    let response = handle_request(&request_json, user_verified);
                    deliver(window_id, webview_id, &request_json, response);
                },
            ));
        });
    }
}

#[cfg(target_os = "macos")]
fn deliver(
    window_id: twarpui::WindowId,
    webview_id: crate::BrowserWebViewId,
    request_json: &str,
    response: Value,
) {
    // The request id is echoed back so the page resolves the right promise.
    let request_id = serde_json::from_str::<Value>(request_json)
        .ok()
        .and_then(|v| v.get("id").and_then(Value::as_str).map(str::to_owned))
        .unwrap_or_default();
    let script = format!(
        "window.__twarpWebAuthnComplete && window.__twarpWebAuthnComplete({}, {})",
        json!(request_id),
        response
    );
    crate::MacWindow::browser_webview_fire_javascript(window_id, webview_id, &script);
}

/// Processes one bridged request and returns the JSON payload for the page.
fn handle_request(request_json: &str, user_verified: bool) -> Value {
    let outcome = (|| -> WebAuthnResult<Value> {
        let request: BridgeRequest = serde_json::from_str(request_json).map_err(|err| {
            WebAuthnError::not_allowed(format!("malformed passkey request: {err}"))
        })?;
        if !user_verified {
            return Err(WebAuthnError::not_allowed(
                "user verification was cancelled or failed",
            ));
        }
        let rp_id = validate_rp_id(&request)?;
        match request.kind.as_str() {
            "create" => create_credential(&request, &rp_id),
            "get" => get_assertion(&request, &rp_id),
            other => Err(WebAuthnError::not_allowed(format!(
                "unsupported passkey operation: {other}"
            ))),
        }
    })();

    match outcome {
        Ok(credential) => json!({ "ok": true, "credential": credential }),
        Err(err) => json!({ "ok": false, "error": { "name": err.name, "message": err.message } }),
    }
}

/// The relying-party id must equal the origin's host or be a parent domain of
/// it (the WebAuthn "registrable domain suffix" rule, minus the public-suffix
/// list refinement).
fn validate_rp_id(request: &BridgeRequest) -> WebAuthnResult<String> {
    let origin = url::Url::parse(&request.origin).map_err(|_| WebAuthnError {
        name: "SecurityError",
        message: format!("invalid origin: {}", request.origin),
    })?;
    let host = origin.host_str().unwrap_or_default().to_owned();
    let rp_id = request
        .rp_id
        .clone()
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| host.clone());
    if rp_id == host || host.ends_with(&format!(".{rp_id}")) {
        Ok(rp_id)
    } else {
        Err(WebAuthnError {
            name: "SecurityError",
            message: format!("rpId {rp_id} is not a registrable suffix of {host}"),
        })
    }
}

fn create_credential(request: &BridgeRequest, rp_id: &str) -> WebAuthnResult<Value> {
    if let Some(algs) = &request.algs {
        if !algs.is_empty() && !algs.contains(&COSE_ALG_ES256) {
            return Err(WebAuthnError {
                name: "NotSupportedError",
                message: "only ES256 (-7) credentials are supported".to_owned(),
            });
        }
    }

    let mut credentials = load_credentials();
    if let Some(exclude) = &request.exclude_ids {
        if credentials
            .iter()
            .any(|c| c.rp_id == rp_id && exclude.contains(&c.cred_id))
        {
            return Err(WebAuthnError {
                name: "InvalidStateError",
                message: "a passkey for this account already exists".to_owned(),
            });
        }
    }

    let secret = SecretKey::random(&mut rand::rngs::OsRng);
    let cred_id: [u8; 32] = rand::random();
    let cred_id_b64 = URL_SAFE_NO_PAD.encode(cred_id);

    let credential = StoredCredential {
        cred_id: cred_id_b64.clone(),
        rp_id: rp_id.to_owned(),
        user_handle: request.user_id.clone().unwrap_or_default(),
        user_name: request.user_name.clone().unwrap_or_default(),
        private_key: URL_SAFE_NO_PAD.encode(secret.to_bytes()),
    };
    // One passkey per (rp, user handle), like platform authenticators.
    credentials.retain(|c| !(c.rp_id == rp_id && c.user_handle == credential.user_handle));
    credentials.push(credential);
    save_credentials(&credentials)?;

    let public_key = secret.public_key();
    let point = public_key.to_encoded_point(false);
    let auth_data = build_auth_data(
        rp_id,
        FLAG_USER_PRESENT | FLAG_USER_VERIFIED | FLAG_ATTESTED_CREDENTIAL_DATA,
        Some((
            &cred_id,
            cose_public_key(point.x().unwrap(), point.y().unwrap()),
        )),
    );
    let attestation_object = cbor_bytes(&ciborium::Value::Map(vec![
        (
            ciborium::Value::Text("fmt".into()),
            ciborium::Value::Text("none".into()),
        ),
        (
            ciborium::Value::Text("attStmt".into()),
            ciborium::Value::Map(vec![]),
        ),
        (
            ciborium::Value::Text("authData".into()),
            ciborium::Value::Bytes(auth_data.clone()),
        ),
    ]));
    let client_data = client_data_json("webauthn.create", request)?;

    let mut spki = P256_SPKI_PREFIX.to_vec();
    spki.extend_from_slice(point.as_bytes());

    Ok(json!({
        "kind": "create",
        "id": cred_id_b64,
        "clientDataJSON": URL_SAFE_NO_PAD.encode(client_data),
        "authData": URL_SAFE_NO_PAD.encode(&auth_data),
        "attestationObject": URL_SAFE_NO_PAD.encode(attestation_object),
        "publicKey": URL_SAFE_NO_PAD.encode(spki),
        "publicKeyAlg": COSE_ALG_ES256,
    }))
}

fn get_assertion(request: &BridgeRequest, rp_id: &str) -> WebAuthnResult<Value> {
    let credentials = load_credentials();
    let allow = request.allow_ids.as_deref().unwrap_or_default();
    let credential = credentials
        .iter()
        .filter(|c| c.rp_id == rp_id)
        .find(|c| allow.is_empty() || allow.contains(&c.cred_id))
        .ok_or_else(|| {
            WebAuthnError::not_allowed(format!("no passkey saved in twarp for {rp_id}"))
        })?;

    let key_bytes = URL_SAFE_NO_PAD
        .decode(&credential.private_key)
        .map_err(|_| WebAuthnError::not_allowed("stored passkey is corrupt"))?;
    let signing_key = SigningKey::from_slice(&key_bytes)
        .map_err(|_| WebAuthnError::not_allowed("stored passkey is corrupt"))?;

    let auth_data = build_auth_data(rp_id, FLAG_USER_PRESENT | FLAG_USER_VERIFIED, None);
    let client_data = client_data_json("webauthn.get", request)?;
    let client_data_hash = Sha256::digest(&client_data);

    let mut signed_payload = auth_data.clone();
    signed_payload.extend_from_slice(&client_data_hash);
    let signature: Signature = signing_key.sign(&signed_payload);

    Ok(json!({
        "kind": "get",
        "id": credential.cred_id,
        "clientDataJSON": URL_SAFE_NO_PAD.encode(client_data),
        "authData": URL_SAFE_NO_PAD.encode(&auth_data),
        "signature": URL_SAFE_NO_PAD.encode(signature.to_der()),
        "userHandle": credential.user_handle,
    }))
}

fn client_data_json(kind: &str, request: &BridgeRequest) -> WebAuthnResult<Vec<u8>> {
    let challenge = request
        .challenge
        .as_deref()
        .filter(|c| !c.is_empty())
        .ok_or_else(|| WebAuthnError::not_allowed("missing challenge"))?;
    Ok(serde_json::to_vec(&json!({
        "type": kind,
        "challenge": challenge,
        "origin": request.origin,
        "crossOrigin": false,
    }))
    .expect("clientDataJSON serialization cannot fail"))
}

fn build_auth_data(
    rp_id: &str,
    flags: u8,
    attested_credential: Option<(&[u8], Vec<u8>)>,
) -> Vec<u8> {
    let mut data = Sha256::digest(rp_id.as_bytes()).to_vec();
    data.push(flags);
    // Constant zero signature counter, matching passkey platform
    // authenticators (avoids clone-detection lockouts across restores).
    data.extend_from_slice(&0u32.to_be_bytes());
    if let Some((cred_id, cose_key)) = attested_credential {
        data.extend_from_slice(&[0u8; 16]); // all-zero AAGUID
        data.extend_from_slice(&(cred_id.len() as u16).to_be_bytes());
        data.extend_from_slice(cred_id);
        data.extend_from_slice(&cose_key);
    }
    data
}

fn cose_public_key(x: &(impl AsRef<[u8]> + ?Sized), y: &(impl AsRef<[u8]> + ?Sized)) -> Vec<u8> {
    cbor_bytes(&ciborium::Value::Map(vec![
        (1.into(), 2.into()),                                     // kty: EC2
        (3.into(), COSE_ALG_ES256.into()),                        // alg: ES256
        ((-1).into(), 1.into()),                                  // crv: P-256
        ((-2).into(), ciborium::Value::Bytes(x.as_ref().into())), // x
        ((-3).into(), ciborium::Value::Bytes(y.as_ref().into())), // y
    ]))
}

fn cbor_bytes(value: &ciborium::Value) -> Vec<u8> {
    let mut bytes = Vec::new();
    ciborium::ser::into_writer(value, &mut bytes).expect("CBOR encoding cannot fail");
    bytes
}

#[cfg(target_os = "macos")]
fn load_credentials() -> Vec<StoredCredential> {
    security_framework::passwords::get_generic_password(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

#[cfg(target_os = "macos")]
fn save_credentials(credentials: &[StoredCredential]) -> WebAuthnResult<()> {
    let bytes = serde_json::to_vec(credentials).expect("credential serialization cannot fail");
    security_framework::passwords::set_generic_password(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT, &bytes)
        .map_err(|err| WebAuthnError {
            name: "UnknownError",
            message: format!("keychain write failed: {err}"),
        })
}

#[cfg(not(target_os = "macos"))]
fn load_credentials() -> Vec<StoredCredential> {
    Vec::new()
}

#[cfg(not(target_os = "macos"))]
fn save_credentials(_credentials: &[StoredCredential]) -> WebAuthnResult<()> {
    Err(WebAuthnError {
        name: "NotSupportedError",
        message: "passkeys are only supported on macOS".to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(kind: &str, rp_id: Option<&str>) -> String {
        json!({
            "id": "req1",
            "kind": kind,
            "origin": "https://www.notion.so",
            "rpId": rp_id,
            "challenge": URL_SAFE_NO_PAD.encode(b"test-challenge"),
            "userId": URL_SAFE_NO_PAD.encode(b"user-1"),
            "userName": "me@example.com",
            "algs": [-7, -257],
        })
        .to_string()
    }

    #[test]
    fn rejects_unverified_user() {
        let response = handle_request(&request("get", Some("notion.so")), false);
        assert_eq!(response["ok"], false);
        assert_eq!(response["error"]["name"], "NotAllowedError");
    }

    #[test]
    fn rejects_foreign_rp_id() {
        let response = handle_request(&request("create", Some("evil.com")), true);
        assert_eq!(response["ok"], false);
        assert_eq!(response["error"]["name"], "SecurityError");
    }

    #[test]
    fn accepts_parent_domain_rp_id() {
        let parsed: BridgeRequest =
            serde_json::from_str(&request("get", Some("notion.so"))).unwrap();
        assert_eq!(validate_rp_id(&parsed).unwrap(), "notion.so");
    }

    #[test]
    fn rejects_rsa_only_registration() {
        let mut value: Value = serde_json::from_str(&request("create", None)).unwrap();
        value["algs"] = json!([-257]);
        let response = handle_request(&value.to_string(), true);
        assert_eq!(response["ok"], false);
        assert_eq!(response["error"]["name"], "NotSupportedError");
    }

    #[test]
    fn auth_data_layout_is_correct() {
        let cose = cose_public_key(&[1u8; 32], &[2u8; 32]);
        let auth_data = build_auth_data("notion.so", 0x45, Some((&[9u8; 32], cose.clone())));
        assert_eq!(&auth_data[..32], Sha256::digest(b"notion.so").as_slice());
        assert_eq!(auth_data[32], 0x45);
        assert_eq!(&auth_data[33..37], &[0, 0, 0, 0]); // zero counter
        assert_eq!(&auth_data[37..53], &[0u8; 16]); // AAGUID
        assert_eq!(&auth_data[53..55], &[0, 32]); // cred id length
        assert_eq!(&auth_data[55..87], &[9u8; 32]);
        assert_eq!(&auth_data[87..], &cose[..]);
    }

    #[test]
    fn signature_verifies_roundtrip() {
        use p256::ecdsa::signature::Verifier;
        let secret = SecretKey::random(&mut rand::rngs::OsRng);
        let signing_key = SigningKey::from(&secret);
        let verifying_key = *signing_key.verifying_key();
        let payload = b"authdata||clienthash";
        let signature: Signature = signing_key.sign(payload);
        let der = signature.to_der();
        let parsed = Signature::from_der(der.as_bytes()).unwrap();
        assert!(verifying_key.verify(payload, &parsed).is_ok());
    }
}
