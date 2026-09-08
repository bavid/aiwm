//! Headless core daemon. Boots paths + config + logging, then idles until a
//! shutdown signal. From WP-6 on it also serves the loopback HTTP/WS API.

use std::process::ExitCode;

use aiwm_core::{app, Result, CORE_VERSION};

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("aiwm-cored: fatal: {err}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<()> {
    let (app, _log_guard) = app::bootstrap_process()?;

    // Register signal handlers before announcing readiness so a shutdown request
    // that races startup is never lost.
    let shutdown = shutdown::Signals::install()?;

    println!(
        "aiwm-core {CORE_VERSION} ready\n  data dir: {}\n  store:    {}\n  api port: {} (loopback)\nPress Ctrl-C to stop.",
        app.paths.root().display(),
        app.config.store_path.display(),
        app.config.core_api_port,
    );

    let reason = shutdown.recv().await;
    tracing::info!(reason, "shutting down cleanly");
    println!("\nshutdown complete ({reason})");
    Ok(())
}

mod shutdown {
    use aiwm_core::{CoreError, Result};

    /// OS shutdown signals, registered on [`Signals::install`].
    pub struct Signals {
        #[cfg(windows)]
        inner: WindowsSignals,
        #[cfg(not(windows))]
        inner: UnixSignals,
    }

    impl Signals {
        pub fn install() -> Result<Self> {
            Ok(Self {
                #[cfg(windows)]
                inner: WindowsSignals::install()?,
                #[cfg(not(windows))]
                inner: UnixSignals::install()?,
            })
        }

        /// Resolve when the OS asks the process to stop, naming the trigger.
        pub async fn recv(mut self) -> &'static str {
            self.inner.recv().await
        }
    }

    #[cfg(windows)]
    struct WindowsSignals {
        ctrl_c: tokio::signal::windows::CtrlC,
        ctrl_break: tokio::signal::windows::CtrlBreak,
        ctrl_close: tokio::signal::windows::CtrlClose,
        ctrl_shutdown: tokio::signal::windows::CtrlShutdown,
    }

    #[cfg(windows)]
    impl WindowsSignals {
        fn install() -> Result<Self> {
            use tokio::signal::windows;
            Ok(Self {
                ctrl_c: windows::ctrl_c().map_err(CoreError::Io)?,
                ctrl_break: windows::ctrl_break().map_err(CoreError::Io)?,
                ctrl_close: windows::ctrl_close().map_err(CoreError::Io)?,
                ctrl_shutdown: windows::ctrl_shutdown().map_err(CoreError::Io)?,
            })
        }

        async fn recv(&mut self) -> &'static str {
            tokio::select! {
                _ = self.ctrl_c.recv() => "ctrl-c",
                _ = self.ctrl_break.recv() => "ctrl-break",
                _ = self.ctrl_close.recv() => "console-close",
                _ = self.ctrl_shutdown.recv() => "system-shutdown",
            }
        }
    }

    #[cfg(not(windows))]
    struct UnixSignals {
        sigint: tokio::signal::unix::Signal,
        sigterm: tokio::signal::unix::Signal,
    }

    #[cfg(not(windows))]
    impl UnixSignals {
        fn install() -> Result<Self> {
            use tokio::signal::unix::{signal, SignalKind};
            Ok(Self {
                sigint: signal(SignalKind::interrupt()).map_err(CoreError::Io)?,
                sigterm: signal(SignalKind::terminate()).map_err(CoreError::Io)?,
            })
        }

        async fn recv(&mut self) -> &'static str {
            tokio::select! {
                _ = self.sigint.recv() => "SIGINT",
                _ = self.sigterm.recv() => "SIGTERM",
            }
        }
    }
}
