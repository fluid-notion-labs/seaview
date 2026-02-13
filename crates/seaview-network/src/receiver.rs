//! Network receiver for streaming mesh data

use crate::protocol::{MessageType, Protocol, ProtocolError, WireFormat};
use crate::types::MeshFrame;

use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use thiserror::Error;
use tracing::{debug, error, info, trace, warn};

/// Errors that can occur during receive operations
#[derive(Error, Debug)]
pub enum ReceiveError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Protocol error: {0}")]
    Protocol(#[from] ProtocolError),

    #[error("Bind error: {0}")]
    Bind(String),

    #[error("Accept timeout")]
    AcceptTimeout,

    #[error("Receive timeout")]
    ReceiveTimeout,

    #[error("Channel send error")]
    ChannelSend,
}

/// Configuration for the mesh receiver
#[derive(Debug, Clone)]
pub struct ReceiverConfig {
    /// Wire format to use
    pub format: WireFormat,
    /// Maximum message size in bytes
    pub max_message_size: usize,
    /// TCP no-delay setting
    pub tcp_nodelay: bool,
    /// Receive buffer size
    pub recv_buffer_size: Option<usize>,
    /// Read timeout for connections
    pub read_timeout: Option<Duration>,
    /// Accept timeout for new connections
    pub accept_timeout: Option<Duration>,
}

impl Default for ReceiverConfig {
    fn default() -> Self {
        Self {
            format: WireFormat::default(),
            max_message_size: 100 * 1024 * 1024, // 100MB
            tcp_nodelay: true,
            recv_buffer_size: Some(1024 * 1024), // 1MB
            read_timeout: Some(Duration::from_secs(30)),
            accept_timeout: None, // Block by default
        }
    }
}

/// Received mesh data with connection metadata
#[derive(Debug, Clone)]
pub struct ReceivedMesh {
    /// The mesh frame data
    pub frame: MeshFrame,
    /// Source address of the sender
    pub source_addr: std::net::SocketAddr,
    /// Timestamp when received
    pub received_at: std::time::Instant,
}

/// TCP-based mesh data receiver
pub struct MeshReceiver {
    listener: TcpListener,
    protocol: Protocol,
    config: ReceiverConfig,
    frames_received: u64,
    bytes_received: u64,
}

impl MeshReceiver {
    /// Create a new mesh receiver listening on the specified address
    pub fn bind<A: ToSocketAddrs>(addr: A) -> Result<Self, ReceiveError> {
        Self::bind_with_config(addr, ReceiverConfig::default())
    }

    /// Create a new mesh receiver with custom configuration
    pub fn bind_with_config<A: ToSocketAddrs>(
        addr: A,
        config: ReceiverConfig,
    ) -> Result<Self, ReceiveError> {
        let listener = TcpListener::bind(addr)
            .map_err(|e| ReceiveError::Bind(format!("Failed to bind: {e}")))?;

        let local_addr = listener.local_addr()?;
        debug!("Mesh receiver listening on {}", local_addr);

        // Set non-blocking mode if accept timeout is specified
        if config.accept_timeout.is_some() {
            listener.set_nonblocking(true)?;
        }

        let protocol =
            Protocol::new(config.format).with_max_message_size(config.max_message_size);

        Ok(Self {
            listener,
            protocol,
            config,
            frames_received: 0,
            bytes_received: 0,
        })
    }

    /// Get the local address the receiver is bound to
    pub fn local_addr(&self) -> Result<std::net::SocketAddr, ReceiveError> {
        Ok(self.listener.local_addr()?)
    }

    /// Accept a single connection and receive one mesh frame
    pub fn receive_one(&mut self) -> Result<ReceivedMesh, ReceiveError> {
        debug!("Waiting for connection...");

        let (mut stream, addr) = if let Some(timeout) = self.config.accept_timeout {
            // Non-blocking accept with timeout
            let start = std::time::Instant::now();
            loop {
                match self.listener.accept() {
                    Ok(result) => break result,
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        if start.elapsed() > timeout {
                            return Err(ReceiveError::AcceptTimeout);
                        }
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(e) => return Err(e.into()),
                }
            }
        } else {
            // Blocking accept
            self.listener.accept()?
        };

        debug!("Accepted connection from {}", addr);

        // Configure the stream
        stream.set_nodelay(self.config.tcp_nodelay)?;

        // Note: TcpStream doesn't have set_recv_buffer_size method
        // Buffer size would need to be set at socket level using platform-specific APIs
        let _ = self.config.recv_buffer_size;

        if let Some(timeout) = self.config.read_timeout {
            stream.set_read_timeout(Some(timeout))?;
        }

        // Receive mesh frame
        let mesh = self.receive_from_stream(&mut stream, addr)?;

        Ok(mesh)
    }

    /// Receive mesh data from a stream
    fn receive_from_stream(
        &mut self,
        stream: &mut TcpStream,
        source_addr: std::net::SocketAddr,
    ) -> Result<ReceivedMesh, ReceiveError> {
        let received_at = std::time::Instant::now();

        loop {
            let message = self.protocol.read_message(stream)?;
            self.bytes_received += message.size() as u64;

            match message.msg_type {
                MessageType::MeshFrame => {
                    trace!("Received mesh frame message");
                    let frame = self.protocol.deserialize_mesh(&message.payload)?;

                    self.frames_received += 1;

                    debug!(
                        "Received frame {} from {} ({} vertices, {} bytes)",
                        frame.frame_number,
                        source_addr,
                        frame.vertex_count(),
                        message.size()
                    );

                    return Ok(ReceivedMesh {
                        frame,
                        source_addr,
                        received_at,
                    });
                }
                MessageType::Heartbeat => {
                    trace!("Received heartbeat");
                    continue;
                }
                MessageType::EndOfStream => {
                    debug!("Received end-of-stream marker from {}", source_addr);
                    return Err(ReceiveError::Io(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "End of stream",
                    )));
                }
                _ => {
                    warn!("Ignoring unexpected message type: {:?}", message.msg_type);
                    continue;
                }
            }
        }
    }

    /// Run the receiver with a callback for each received mesh
    pub fn run<F>(&mut self, mut callback: F) -> Result<(), ReceiveError>
    where
        F: FnMut(ReceivedMesh) -> bool,
    {
        debug!("Starting mesh receiver loop");

        loop {
            match self.receive_one() {
                Ok(mesh) => {
                    if !callback(mesh) {
                        debug!("Callback requested stop");
                        break;
                    }
                }
                Err(ReceiveError::AcceptTimeout) => {
                    trace!("Accept timeout, continuing");
                    continue;
                }
                Err(e) => {
                    error!("Error receiving mesh: {}", e);
                    // Continue on most errors
                    if !matches!(e, ReceiveError::Io(_) | ReceiveError::Protocol(_)) {
                        continue;
                    }
                }
            }
        }

        debug!(
            "Mesh receiver stopped. Received {} frames, {} bytes",
            self.frames_received, self.bytes_received
        );

        Ok(())
    }

    /// Start receiver in a background thread with a channel
    pub fn run_async(mut self) -> (mpsc::Receiver<ReceivedMesh>, thread::JoinHandle<()>) {
        let (tx, rx) = mpsc::channel();

        let handle = thread::spawn(move || {
            let _ = self.run(|mesh| tx.send(mesh).is_ok());
        });

        (rx, handle)
    }

    /// Get statistics about received data
    pub fn stats(&self) -> ReceiverStats {
        ReceiverStats {
            frames_received: self.frames_received,
            bytes_received: self.bytes_received,
        }
    }
}

/// Non-blocking mesh receiver that can be polled
pub struct NonBlockingMeshReceiver {
    listener: TcpListener,
    protocol: Protocol,
    config: ReceiverConfig,
}

impl NonBlockingMeshReceiver {
    /// Create a new non-blocking mesh receiver
    pub fn bind<A: ToSocketAddrs>(addr: A) -> Result<Self, ReceiveError> {
        Self::bind_with_config(addr, ReceiverConfig::default())
    }

    /// Create a new non-blocking mesh receiver with custom configuration
    pub fn bind_with_config<A: ToSocketAddrs>(
        addr: A,
        config: ReceiverConfig,
    ) -> Result<Self, ReceiveError> {
        let listener = TcpListener::bind(addr)
            .map_err(|e| ReceiveError::Bind(format!("Failed to bind: {e}")))?;

        let local_addr = listener.local_addr()?;
        debug!("Non-blocking mesh receiver listening on {}", local_addr);

        // Always set non-blocking for this receiver type
        listener.set_nonblocking(true)?;

        let protocol =
            Protocol::new(config.format).with_max_message_size(config.max_message_size);

        Ok(Self {
            listener,
            protocol,
            config,
        })
    }

    /// Try to receive a mesh without blocking
    pub fn try_receive(&mut self) -> Result<Option<ReceivedMesh>, ReceiveError> {
        match self.listener.accept() {
            Ok((mut stream, addr)) => {
                debug!("Accepted connection from {}", addr);

                // Configure the stream
                stream.set_nodelay(self.config.tcp_nodelay)?;

                // Note: TcpStream doesn't have set_recv_buffer_size method
                // Buffer size would need to be set at socket level using platform-specific APIs
                let _ = self.config.recv_buffer_size;

                if let Some(timeout) = self.config.read_timeout {
                    stream.set_read_timeout(Some(timeout))?;
                }

                // Switch to blocking mode for reading
                stream.set_nonblocking(false)?;

                // Try to receive mesh
                match self.receive_from_stream(&mut stream, addr) {
                    Ok(mesh) => Ok(Some(mesh)),
                    Err(e) => Err(e),
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Receive mesh data from a stream
    fn receive_from_stream(
        &mut self,
        stream: &mut TcpStream,
        source_addr: std::net::SocketAddr,
    ) -> Result<ReceivedMesh, ReceiveError> {
        let received_at = std::time::Instant::now();

        loop {
            let message = self.protocol.read_message(stream)?;

            match message.msg_type {
                MessageType::MeshFrame => {
                    let frame = self.protocol.deserialize_mesh(&message.payload)?;

                    debug!(
                        "Received frame {} from {} ({} vertices)",
                        frame.frame_number,
                        source_addr,
                        frame.vertex_count()
                    );

                    return Ok(ReceivedMesh {
                        frame,
                        source_addr,
                        received_at,
                    });
                }
                MessageType::Heartbeat => {
                    trace!("Received heartbeat");
                    continue;
                }
                MessageType::EndOfStream => {
                    debug!("Received end-of-stream marker from {}", source_addr);
                    return Err(ReceiveError::Io(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "End of stream",
                    )));
                }
                _ => {
                    warn!("Ignoring unexpected message type: {:?}", message.msg_type);
                    continue;
                }
            }
        }
    }

    /// Get the local address
    pub fn local_addr(&self) -> Result<std::net::SocketAddr, ReceiveError> {
        Ok(self.listener.local_addr()?)
    }
}

/// Statistics about received data
#[derive(Debug, Clone, Copy)]
pub struct ReceiverStats {
    /// Number of frames received
    pub frames_received: u64,
    /// Total bytes received
    pub bytes_received: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_receiver_config_default() {
        let config = ReceiverConfig::default();
        assert!(config.tcp_nodelay);
        assert_eq!(config.max_message_size, 100 * 1024 * 1024);
    }

    #[test]
    fn test_receiver_bind() {
        let receiver = MeshReceiver::bind("127.0.0.1:0");
        assert!(receiver.is_ok());

        let receiver = receiver.unwrap();
        assert!(receiver.local_addr().is_ok());
    }

    #[test]
    fn test_non_blocking_receiver() {
        let mut receiver = NonBlockingMeshReceiver::bind("127.0.0.1:0").unwrap();

        // Should return None when no connection
        let result = receiver.try_receive();
        assert!(matches!(result, Ok(None)));
    }

    #[test]
    fn test_receiver_stats() {
        let receiver = MeshReceiver::bind("127.0.0.1:0").unwrap();
        let stats = receiver.stats();
        assert_eq!(stats.frames_received, 0);
        assert_eq!(stats.bytes_received, 0);
    }
}
