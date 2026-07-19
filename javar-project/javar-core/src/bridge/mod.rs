//! Communication bridges to the Java Agent and IDE clients.

mod socket;

pub use socket::SocketBridge;

use crate::protocol::Message;
use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct BridgeConfig {
    pub addr: String,
    pub reconnect: bool,
}

/// Abstraction over socket / JNI delivery paths.
#[async_trait]
pub trait AgentBridge: Send + Sync {
    async fn send(&self, msg: Message) -> Result<()>;
    async fn close(&self) -> Result<()>;
}

/// Shared handle type used by the core runtime.
pub type SharedBridge = Arc<dyn AgentBridge>;

#[cfg(feature = "jni-bridge")]
pub mod jni_bridge {
    use super::*;
    use jni::objects::{JClass, JObject, JValue};
    use jni::JNIEnv;
    use parking_lot::Mutex;
    use std::sync::OnceLock;

    /// Global JVM pointer registered from the Java Agent via JNI_OnLoad / native hook.
    static JVM: OnceLock<jni::JavaVM> = OnceLock::new();

    pub fn register_jvm(vm: jni::JavaVM) {
        let _ = JVM.set(vm);
    }

    pub struct JniBridge {
        agent_class: Mutex<Option<String>>,
    }

    impl JniBridge {
        pub fn new() -> Self {
            Self {
                agent_class: Mutex::new(Some("com/javar/agent/JavaRAgent".into())),
            }
        }
    }

    #[async_trait]
    impl AgentBridge for JniBridge {
        async fn send(&self, msg: Message) -> Result<()> {
            // JNI calls must run on an attached thread; keep this path for Phase-1.5.
            let vm = JVM
                .get()
                .ok_or_else(|| anyhow::anyhow!("JVM not registered for JNI bridge"))?;
            let mut env = vm.attach_current_thread()?;
            let class_name = self
                .agent_class
                .lock()
                .clone()
                .unwrap_or_else(|| "com/javar/agent/JavaRAgent".into());
            let class: JClass = env.find_class(&class_name)?;
            let frame = crate::protocol::Frame::encode(&msg);
            let bytes = frame.to_bytes();
            let jbytes = env.byte_array_from_slice(&bytes)?;
            env.call_static_method(
                class,
                "onNativeFrame",
                "([B)V",
                &[JValue::Object(&JObject::from(jbytes))],
            )?;
            Ok(())
        }

        async fn close(&self) -> Result<()> {
            Ok(())
        }
    }
}
