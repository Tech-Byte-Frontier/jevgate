//! Secret Service reads and deletion never invoke its Unlock or Prompt methods.
//! The higher-level keyring library can open a dialog even when just reading.
use anyhow::{Result, ensure};
use secret_service::{EncryptionType, blocking::SecretService};
use std::collections::HashMap;
use zbus::blocking::{Connection, Proxy};
use zeroize::Zeroizing;

fn attributes() -> HashMap<&'static str, &'static str> {
    HashMap::from([("service", "jevgate"), ("username", "typesafe-api-key")])
}

fn unlocked_item<'a>(
    service: &'a SecretService<'a>,
) -> Result<Option<secret_service::blocking::Item<'a>>> {
    let mut items = service.search_items(attributes())?;
    ensure!(items.locked.is_empty(), "locked");
    ensure!(items.unlocked.len() <= 1, "ambiguous");
    Ok(items.unlocked.pop())
}

/// The saved credential's text, as `store` wrote it.
pub fn get() -> Result<Option<Zeroizing<String>>> {
    (|| -> Result<Option<Zeroizing<String>>> {
        let service = SecretService::connect(EncryptionType::Dh)?;
        let Some(item) = unlocked_item(&service)? else {
            return Ok(None);
        };
        let bytes = Zeroizing::new(item.get_secret()?);
        let value = std::str::from_utf8(&bytes)?;
        Ok(Some(Zeroizing::new(value.to_owned())))
    })()
    .map_err(|_| anyhow::anyhow!(
        "Cannot read the system credential store; unlock it and retry, or use TYPESAFE_API_KEY or an --env-file. Run jevgate auth login to configure credentials"
    ))
}

pub fn delete() -> Result<bool> {
    (|| -> Result<bool> {
        let connection = Connection::session()?;
        let service = SecretService::connect_with_existing(EncryptionType::Dh, connection.clone())?;
        let Some(item) = unlocked_item(&service)? else {
            return Ok(false);
        };
        let proxy = Proxy::new(
            &connection,
            "org.freedesktop.secrets",
            item.item_path.as_str(),
            "org.freedesktop.Secret.Item",
        )?;
        let prompt: zbus::zvariant::OwnedObjectPath = proxy.call("Delete", &())?;
        // Do not dispatch Prompt even if the service requires confirmation.
        ensure!(prompt.as_str() == "/", "confirmation required");
        Ok(true)
    })()
    .map_err(|_| anyhow::anyhow!(
        "Cannot remove the system credential without a dialog; unlock the store or remove the jevgate entry in your system credential manager"
    ))
}
