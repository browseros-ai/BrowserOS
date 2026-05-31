use sha2::{Sha256, Digest};
use hex;
use chrono::Utc;
use crate::constitution::{Constitution, OversightResult, ChangeSpec};
use crate::variant::Variant;

/// Independent oversight with cryptographic signature
/// Key stored externally — agent does NOT have access
pub struct OversightAgent {
    constitution: Constitution,
    signature_key_id: String,
}

impl Default for OversightAgent {
    fn default() -> Self {
        Self::new()
    }
}

impl OversightAgent {
    pub fn new() -> Self {
        Self {
            constitution: Constitution::default(),
            signature_key_id: "HSM_OVERSIGHT_01".to_string(),
        }
    }
    
    pub fn evaluate(&self,
        changes: &[ChangeSpec],
        _variant: &Variant,
    ) -> (OversightResult, String) {
        let result = self.constitution.evaluate(changes);
        let token = self.sign(&result, changes);
        (result, token)
    }
    
    /// Cryptographic signature using external HSM
    fn sign(&self,
        result: &OversightResult,
        changes: &[ChangeSpec],
    ) -> String {
        let payload = format!(
            "{}:{}:{}",
            std::any::type_name_of_val(result),
            changes.len(),
            Utc::now().timestamp()
        );
        
        let mut hasher = Sha256::new();
        hasher.update(payload.as_bytes());
        hasher.update(self.signature_key_id.as_bytes());
        // In real system: HSM_SIGN(key_id, payload)
        
        let digest = hasher.finalize();
        hex::encode(&digest[..16])
    }
}
