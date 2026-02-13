//! Network sender for streaming mesh data

use crate::protocol::{Protocol, ProtocolError, WireFormat};
use crate::types::MeshFrame;
use std::io::Write;
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;
use thiserror::Error;
use tracing::{debug, error, info, trace};

/// Errors that can occur during network operations
#[derive(Error, Debug)]
pub enum NetworkError {
    #[error("Connection error: {0}")]
    Connection(#[from] std::io::Error),

    #[error("Protocol error: {0}")]
    Protocol(#[from] ProtocolError),

    #[error("Connection closed by remote")]
    ConnectionClosed,

    #[error("Send timeout after {0:?}")]
    SendTimeout(Duration),

    #[error("Invalid address: {0}")]
    InvalidAddress(String),
}

/// Configuration for the mesh sender
#[derive(Debug, Clone)]
pub struct SenderConfig {
    /// Wire format to use
    pub format: WireFormat,
    /// Maximum message size in bytes
    pub max_message_size: usize,
    /// TCP no-delay setting
    pub tcp_nodelay: bool,
    /// Send buffer size
    pub send_buffer_size: Option<usize>,
    /// Connection timeout
    pub connect_timeout: Option<Duration>,
    /// Write timeout
    pub write_timeout: Option<Duration>,
}

impl Default for SenderConfig {
    fn default() -> Self {
        Self {
            format: WireFormat::default(),
            max_message_size: 100 * 1024 * 1024, // 100MB
            tcp_nodelay: true,
            send_buffer_size: Some(1024 * 1024), // 1MB
            connect_timeout: Some(Duration::from_secs(10)),
            write_timeout: Some(Duration::from_secs(30)),
        }
    }
}

/// TCP-based mesh data sender
pub struct MeshSender {
    stream: TcpStream,
    protocol: Protocol,
    _config: SenderConfig,
    frames_sent: u64,
    bytes_sent: u64,
}

impl MeshSender {
    /// Create a new mesh sender connected to the specified address
    pub fn connect<A: ToSocketAddrs>(addr: A) -> Result<Self, NetworkError> {
        Self::connect_with_config(addr, SenderConfig::default())
    }

    /// Create a new mesh sender with custom configuration
    pub fn connect_with_config<A: ToSocketAddrs>(
        addr: A,
        config: SenderConfig,
    ) -> Result<Self, NetworkError> {
        debug!("Connecting to mesh receiver...");

        let stream = if let Some(timeout) = config.connect_timeout {
            // Convert address to SocketAddr for timeout connection
            let socket_addr = addr
                .to_socket_addrs()?
                .next()
                .ok_or_else(|| NetworkError::InvalidAddress("No valid address".to_string()))?;

            TcpStream::connect_timeout(&socket_addr, timeout)?
        } else {
            TcpStream::connect(addr)?
        };

        debug!("Connected to {}", stream.peer_addr()?);

        // Configure the stream
        stream.set_nodelay(config.tcp_nodelay)?;

        // Note: TcpStream doesn't have set_send_buffer_size method
        // Buffer size would need to be set at socket level using platform-specific APIs
        let _ = config.send_buffer_size;

        if let Some(timeout) = config.write_timeout {
            stream.set_write_timeout(Some(timeout))?;
        }

        let protocol = Protocol::new(config.format).with_max_message_size(config.max_message_size);

        Ok(Self {
            stream,
            protocol,
            _config: config,
            frames_sent: 0,
            bytes_sent: 0,
        })
    }

    /// Send a mesh frame
    pub fn send_mesh(&mut self, mesh: &MeshFrame) -> Result<(), NetworkError> {
        trace!(
            "Sending mesh frame: sim_id={}, frame={}, vertices={}",
            mesh.simulation_id,
            mesh.frame_number,
            mesh.vertex_count()
        );

        // Validate mesh data
        if let Err(e) = mesh.validate() {
            error!("Invalid mesh data: {}", e);
            return Err(NetworkError::Protocol(ProtocolError::InvalidFormat));
        }

        // Serialize the mesh
        let message = self.protocol.serialize_mesh(mesh)?;
        let message_size = message.size();

        // Send the message
        self.protocol.write_message(&mut self.stream, &message)?;

        self.frames_sent += 1;
        self.bytes_sent += message_size as u64;

        debug!(
            "Sent frame {} ({} bytes, total: {} frames, {} bytes)",
            mesh.frame_number, message_size, self.frames_sent, self.bytes_sent
        );

        Ok(())
    }

    /// Send a heartbeat message
    pub fn send_heartbeat(&mut self) -> Result<(), NetworkError> {
        trace!("Sending heartbeat");
        let message = self.protocol.create_heartbeat();
        self.protocol.write_message(&mut self.stream, &message)?;
        Ok(())
    }

    /// Send end-of-stream marker
    pub fn send_end_of_stream(&mut self) -> Result<(), NetworkError> {
        debug!("Sending end-of-stream marker");
        let message = self.protocol.create_end_of_stream();
        self.protocol.write_message(&mut self.stream, &message)?;
        Ok(())
    }

    /// Flush any buffered data
    pub fn flush(&mut self) -> Result<(), NetworkError> {
        self.stream.flush()?;
        Ok(())
    }

    /// Get statistics about sent data
    pub fn stats(&self) -> SenderStats {
        SenderStats {
            frames_sent: self.frames_sent,
            bytes_sent: self.bytes_sent,
        }
    }

    /// Get the peer address
    pub fn peer_addr(&self) -> Result<std::net::SocketAddr, NetworkError> {
        Ok(self.stream.peer_addr()?)
    }

    /// Get the local address
    pub fn local_addr(&self) -> Result<std::net::SocketAddr, NetworkError> {
        Ok(self.stream.local_addr()?)
    }

    /// Set TCP no-delay option
    pub fn set_nodelay(&self, nodelay: bool) -> Result<(), NetworkError> {
        self.stream.set_nodelay(nodelay)?;
        Ok(())
    }

    /// Shutdown the connection gracefully
    pub fn shutdown(mut self) -> Result<(), NetworkError> {
        debug!("Shutting down mesh sender");

        // Try to send end-of-stream marker
        let _ = self.send_end_of_stream();

        // Flush any remaining data
        let _ = self.stream.flush();

        // Shutdown the TCP connection
        self.stream.shutdown(std::net::Shutdown::Both)?;

        debug!(
            "Mesh sender shutdown complete. Sent {} frames, {} bytes",
            self.frames_sent, self.bytes_sent
        );

        Ok(())
    }
}

/// Statistics about sent data
#[derive(Debug, Clone, Copy)]
pub struct SenderStats {
    /// Number of frames sent
    pub frames_sent: u64,
    /// Total bytes sent
    pub bytes_sent: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn test_sender_config_default() {
        let config = SenderConfig::default();
        assert!(config.tcp_nodelay);
        assert_eq!(config.max_message_size, 100 * 1024 * 1024);
    }

    #[test]
    fn test_sender_connect_fail() {
        // Should fail to connect to invalid address
        let result = MeshSender::connect("0.0.0.0:0");
        assert!(result.is_err());
    }

    #[test]
    fn test_sender_connect_success() {
        // Start a listener
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        // Accept connections in background
        thread::spawn(move || {
            let _ = listener.accept();
        });

        // Connect should succeed
        let sender = MeshSender::connect(addr);
        assert!(sender.is_ok());
    }

    #[test]
    fn test_sender_stats() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        thread::spawn(move || {
            let _ = listener.accept();
        });

        let sender = MeshSender::connect(addr).unwrap();
        let stats = sender.stats();
        assert_eq!(stats.frames_sent, 0);
        assert_eq!(stats.bytes_sent, 0);
    }
}
