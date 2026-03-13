use shared::async_nats::{self, ConnectErrorKind};
use shared::log::SetLoggerError;
use std::error;
use std::fmt;
use std::io;
use std::time::SystemTimeError;

#[derive(Debug, Clone, Copy)]
pub enum IpcCallKind {
    InitConstruct,
    ThreadMapMakeThread,
    InitMakeMining,
    MiningGetTip,
}

impl fmt::Display for IpcCallKind {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            IpcCallKind::InitConstruct => write!(f, "init.construct"),
            IpcCallKind::ThreadMapMakeThread => write!(f, "thread_map.make_thread"),
            IpcCallKind::InitMakeMining => write!(f, "init.make_mining"),
            IpcCallKind::MiningGetTip => write!(f, "mining.get_tip"),
        }
    }
}

#[derive(Debug)]
pub enum RuntimeError {
    SetLogger(SetLoggerError),
    Io(io::Error),
    IpcCall {
        kind: IpcCallKind,
        source: capnp::Error,
    },
    SystemTime(SystemTimeError),
    NatsConnect(async_nats::error::Error<ConnectErrorKind>),
    NatsPublish(async_nats::error::Error<async_nats::client::PublishErrorKind>),
}

impl RuntimeError {
    pub fn ipc_call(kind: IpcCallKind, source: capnp::Error) -> Self {
        RuntimeError::IpcCall { kind, source }
    }
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            RuntimeError::SetLogger(e) => write!(f, "set logger error {}", e),
            RuntimeError::Io(e) => write!(f, "IO error {}", e),
            RuntimeError::IpcCall { kind, source } => write!(f, "IPC {} error {}", kind, source),
            RuntimeError::SystemTime(e) => write!(f, "system time error {}", e),
            RuntimeError::NatsConnect(e) => write!(f, "NATS connection error {}", e),
            RuntimeError::NatsPublish(e) => write!(f, "NATS publish error {}", e),
        }
    }
}

impl error::Error for RuntimeError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match *self {
            RuntimeError::SetLogger(ref e) => Some(e),
            RuntimeError::Io(ref e) => Some(e),
            RuntimeError::IpcCall { ref source, .. } => Some(source),
            RuntimeError::SystemTime(ref e) => Some(e),
            RuntimeError::NatsConnect(ref e) => Some(e),
            RuntimeError::NatsPublish(ref e) => Some(e),
        }
    }
}

impl From<SetLoggerError> for RuntimeError {
    fn from(e: SetLoggerError) -> Self {
        RuntimeError::SetLogger(e)
    }
}

impl From<io::Error> for RuntimeError {
    fn from(e: io::Error) -> Self {
        RuntimeError::Io(e)
    }
}

impl From<SystemTimeError> for RuntimeError {
    fn from(e: SystemTimeError) -> Self {
        RuntimeError::SystemTime(e)
    }
}

impl From<async_nats::error::Error<ConnectErrorKind>> for RuntimeError {
    fn from(e: async_nats::error::Error<ConnectErrorKind>) -> Self {
        RuntimeError::NatsConnect(e)
    }
}

impl From<async_nats::error::Error<async_nats::client::PublishErrorKind>> for RuntimeError {
    fn from(e: async_nats::error::Error<async_nats::client::PublishErrorKind>) -> Self {
        RuntimeError::NatsPublish(e)
    }
}
