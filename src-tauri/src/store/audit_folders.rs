//! The model the audit runs on and who pays for it, and the old `audit-folders.json` (one
//! project folder per connection), which `code_sources` now replaces: its folders become code
//! sources on first read, and a deleted connection's entry is still removed from it.

use super::Store;
use crate::audit::runner::{AuditModel, AuditPayment};
use crate::error::AppResult;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const FILE: &str = "audit-folders.json";
const PREFS: &str = "audit-settings.json";

#[derive(Default, Serialize, Deserialize)]
struct AuditPrefs {
    #[serde(default)]
    model: AuditModel,
    #[serde(default)]
    payment: AuditPayment,
}

impl Store {
    /// The model the audit run uses (Sonnet until the person picks another).
    pub fn audit_model(&self) -> AppResult<AuditModel> {
        Ok(self.read::<AuditPrefs>(PREFS)?.model)
    }

    pub fn set_audit_model(&self, model: AuditModel) -> AppResult<()> {
        self.update(PREFS, |prefs: &mut AuditPrefs| {
            prefs.model = model;
            Ok(())
        })
    }

    /// Who pays for the audit run: the person's own sign-in unless they picked credits (a saved
    /// choice that is ignored while `audit::runner::CREDITS_RUNS` is off).
    pub fn audit_payment(&self) -> AppResult<AuditPayment> {
        Ok(self.read::<AuditPrefs>(PREFS)?.payment)
    }

    pub fn set_audit_payment(&self, payment: AuditPayment) -> AppResult<()> {
        self.update(PREFS, |prefs: &mut AuditPrefs| {
            prefs.payment = payment;
            Ok(())
        })
    }

    /// Forgets a deleted connection's folder.
    pub(super) fn forget_audit_folder(&self, connection_id: &str) -> AppResult<()> {
        self.update(FILE, |all: &mut BTreeMap<String, String>| {
            all.remove(connection_id);
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_deleted_connection_leaves_the_old_folders_file() {
        let dir = std::env::temp_dir().join(format!("sayql-folders-{}", uuid::Uuid::new_v4()));
        let s = Store::new(dir.clone());
        std::fs::write(dir.join(FILE), r#"{"c1":"/a","c2":"/b"}"#).unwrap();
        s.forget_agent("c1").unwrap();
        let left: BTreeMap<String, String> = s.read(FILE).unwrap();
        assert_eq!(left.keys().collect::<Vec<_>>(), ["c2"]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn remembers_the_audit_model() {
        let dir = std::env::temp_dir().join(format!("sayql-audit-model-{}", uuid::Uuid::new_v4()));
        let s = Store::new(dir.clone());
        assert_eq!(s.audit_model().unwrap(), AuditModel::Sonnet);
        s.set_audit_model(AuditModel::Default).unwrap();
        assert_eq!(s.audit_model().unwrap(), AuditModel::Default);
        assert_eq!(
            Store::new(dir.clone()).audit_model().unwrap(),
            AuditModel::Default
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn remembers_who_pays_next_to_the_model() {
        let dir = std::env::temp_dir().join(format!("sayql-audit-pay-{}", uuid::Uuid::new_v4()));
        let s = Store::new(dir.clone());
        // Settings saved before payment existed read as the person's own sign-in.
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(PREFS), r#"{"model":"opus"}"#).unwrap();
        assert_eq!(s.audit_payment().unwrap(), AuditPayment::Subscription);
        s.set_audit_payment(AuditPayment::Credits).unwrap();
        let again = Store::new(dir.clone());
        assert_eq!(again.audit_payment().unwrap(), AuditPayment::Credits);
        assert_eq!(again.audit_model().unwrap(), AuditModel::Opus);
        let _ = std::fs::remove_dir_all(dir);
    }
}
