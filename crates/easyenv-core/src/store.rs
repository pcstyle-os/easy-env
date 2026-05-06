use anyhow::Result;

use crate::SecretLocator;

pub trait SecretStore {
    fn put(&self, locator: &SecretLocator, secret: &[u8]) -> Result<()>;
    fn get(&self, locator: &SecretLocator) -> Result<Option<Vec<u8>>>;
    fn delete(&self, locator: &SecretLocator) -> Result<()>;
    fn probe(&self) -> Result<()>;
}
