//! Interactive terminal input, output, resizing, and socket pumping.
use super::*;

pub(super) enum Keystrokes {
    Data(Bytes),
    Control(wire::Session),
}

pub(super) struct RunnerPty {
    pub(super) output: mpsc::Receiver<Result<Bytes>>,
    pub(super) input: mpsc::Sender<Keystrokes>,
}

#[async_trait::async_trait]
impl Pty for RunnerPty {
    async fn write(&mut self, data: &[u8]) -> Result<()> {
        self.input
            .send(Keystrokes::Data(Bytes::copy_from_slice(data)))
            .await
            .map_err(|_| runner_extra("runnerExtra.terminalEnded"))
    }

    async fn read(&mut self) -> Result<Option<Bytes>> {
        match self.output.recv().await {
            Some(Ok(bytes)) => Ok(Some(bytes)),
            Some(Err(error)) => Err(error),
            None => Ok(None),
        }
    }

    async fn resize(&mut self, size: PtySize) -> Result<()> {
        let size = size.sane();
        self.input
            .send(Keystrokes::Control(wire::Session::TerminalResize {
                cols: size.cols,
                rows: size.rows,
            }))
            .await
            .map_err(|_| runner_extra("runnerExtra.terminalEnded"))
    }
}

pub(super) async fn pump_pty(
    mut socket: Socket,
    output: mpsc::Sender<Result<Bytes>>,
    mut input: mpsc::Receiver<Keystrokes>,
) {
    loop {
        tokio::select! {
            incoming = socket.recv() => {
                let Some(Ok(message)) = incoming else { break };
                match message {
                    Message::Binary(bytes) => if output.send(Ok(bytes)).await.is_err() { break },
                    Message::Text(text) => match serde_json::from_str::<wire::Session>(&text) {
                        Ok(wire::Session::Eof) | Ok(wire::Session::Done) => break,
                        Ok(wire::Session::Failed { error }) => {
                            let _ = output.send(Err(remote_failed(error))).await;
                            break;
                        }
                        _ => {}
                    },
                    Message::Close(_) => break,
                    _ => {}
                }
            }
            request = input.recv() => match request {
                Some(Keystrokes::Data(bytes)) => {
                    if socket.send(Message::Binary(bytes)).await.is_err() { break }
                }
                Some(Keystrokes::Control(frame)) => {
                    if send_frame(&mut socket, &frame).await.is_err() { break }
                }
                // The `Pty` was dropped, which closes the remote session.
                None => break,
            },
        }
    }
    let _ = socket.send(Message::Close(None)).await;
}
