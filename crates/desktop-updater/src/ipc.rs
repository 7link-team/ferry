//! Local Windows pipe transport. Peer identity comes from the kernel, never JSON.
use std::path::Path;
use tokio::net::windows::named_pipe::{NamedPipeClient, NamedPipeServer};

pub struct Process;
impl Process {
    pub fn open(_pid: u32) -> anyhow::Result<Self> {
        anyhow::bail!("process identity not implemented")
    }
    pub fn pid(&self) -> u32 {
        0
    }
    pub fn matches_image(&self, _path: &Path) -> anyhow::Result<bool> {
        Ok(false)
    }
}

pub fn listen() -> anyhow::Result<NamedPipeServer> {
    create(&pipe_name())
}
pub fn pipe_name() -> String {
    format!(r"\\.\pipe\{}.desktop-update", env!("DESKTOP_IDENTIFIER"))
}
fn create(_name: &str) -> anyhow::Result<NamedPipeServer> {
    anyhow::bail!("pipe listener not implemented")
}
fn connect_peer(_name: &str, _expected: &Process) -> anyhow::Result<NamedPipeClient> {
    anyhow::bail!("pipe client not implemented")
}
pub fn client_process(_server: &NamedPipeServer) -> anyhow::Result<Process> {
    anyhow::bail!("pipe peer identity not implemented")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    #[tokio::test]
    async fn exclusive_local_pipe_authenticates_both_peer_processes() {
        let name = format!("{}-test-{}", pipe_name(), std::process::id());
        let process = Process::open(std::process::id()).unwrap();
        assert!(
            process
                .matches_image(&std::env::current_exe().unwrap())
                .unwrap()
        );
        assert!(
            !process
                .matches_image(Path::new(r"C:\not-the-updater.exe"))
                .unwrap()
        );
        let mut server = create(&name).unwrap();
        assert!(
            create(&name).is_err(),
            "second server must not impersonate the first"
        );
        let mut client = connect_peer(&name, &process).unwrap();
        server.connect().await.unwrap();
        assert_eq!(client_process(&server).unwrap().pid(), std::process::id());
        client.write_all(b"request").await.unwrap();
        let mut bytes = [0; 7];
        server.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"request");
        server.write_all(b"response").await.unwrap();
        let mut bytes = [0; 8];
        client.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"response");
    }
}
