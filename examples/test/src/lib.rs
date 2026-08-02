//! test Pumpkin mod.

use std::{ops::{Deref, DerefMut}, sync::Arc};
use pumpkin::{plugin::Context};
use pumpkin_api_macros::{plugin_impl, plugin_method};

async fn on_load_inner(plugin: &Plugin, context: Arc<Context>) -> Result<(), String> {
    context.init_log();
    tracing::info!("Loading test");

    {
        let mut guard = context.server.test_data.write().await;
        (*guard.deref_mut()) = 10;
    }

    {
        let guard = context.server.test_data.read().await;
        tracing::info!("Read {} from test_data", guard.deref());
    }

    Ok(())
}

async fn on_unload_inner(plugin: &Plugin, context: Arc<Context>) -> Result<(), String> {
    tracing::info!("Unloading test");
    Ok(())
}

///
/// WARNING! All plugin_methods must appear before plugin_impl
/// 

#[plugin_method]
async fn on_load(&self, context: Arc<Context>) -> Result<(), String> {
    on_load_inner(self, context).await
}

#[plugin_method]
async fn on_unload(&self, context: Arc<Context>) -> Result<(), String> {
    on_unload_inner(self, context).await
}

#[plugin_impl]
pub struct Plugin;

impl Plugin {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Default for Plugin {
    fn default() -> Self {
        Self::new()
    }
}
