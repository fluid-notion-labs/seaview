//! FFI bindings for C/C++ integration
//!
//! This module provides a C-compatible API for using the seaview-network library
//! from C and C++ applications.

use crate::protocol::WireFormat;
use crate::sender::{MeshSender, SenderConfig};
use crate::types::{DomainBounds, MeshFrame};
use std::ffi::{c_char, CStr};
use std::os::raw::{c_float, c_int, c_uint};
use std::ptr;
use std::slice;
use std::time::Duration;
use tracing::{debug, error, info};

/// Opaque handle to a network sender
pub struct NetworkSender {
    sender: MeshSender,
}

/// C-compatible mesh frame structure
#[repr(C)]
pub struct CMeshFrame {
    /// Null-terminated simulation ID string
    pub simulation_id: *const c_char,
    /// Frame number
    pub frame_number: c_uint,
    /// Timestamp in nanoseconds
    pub timestamp: u64,
    /// Domain minimum bounds (x, y, z)
    pub domain_min: [c_float; 3],
    /// Domain maximum bounds (x, y, z)
    pub domain_max: [c_float; 3],
    /// Number of vertices (must be divisible by 3 for triangle soup)
    pub vertex_count: usize,
    /// Pointer to vertex data (x,y,z triplets)
    pub vertices: *const c_float,
    /// Pointer to normal data (x,y,z triplets), NULL if no normals
    pub normals: *const c_float,
    /// Number of indices, 0 if not indexed
    pub index_count: usize,
    /// Pointer to index data, NULL if not indexed
    pub indices: *const c_uint,
}

/// Wire format options
#[repr(C)]
pub enum CWireFormat {
    /// Binary format (default)
    Bincode = 0,
    /// JSON format (if feature enabled)
    Json = 1,
}

/// Sender configuration
#[repr(C)]
pub struct CSenderConfig {
    /// Wire format to use
    pub format: CWireFormat,
    /// Maximum message size in bytes
    pub max_message_size: usize,
    /// Enable TCP no-delay (1 = true, 0 = false)
    pub tcp_nodelay: c_int,
    /// Send buffer size in bytes (0 = system default)
    pub send_buffer_size: usize,
    /// Connection timeout in milliseconds (0 = no timeout)
    pub connect_timeout_ms: c_uint,
    /// Write timeout in milliseconds (0 = no timeout)
    pub write_timeout_ms: c_uint,
}

/// Create a default sender configuration
#[no_mangle]
pub extern "C" fn seaview_network_default_config() -> CSenderConfig {
    CSenderConfig {
        format: CWireFormat::Bincode,
        max_message_size: 100 * 1024 * 1024, // 100MB
        tcp_nodelay: 1,
        send_buffer_size: 1024 * 1024, // 1MB
        connect_timeout_ms: 10000,     // 10 seconds
        write_timeout_ms: 30000,       // 30 seconds
    }
}

/// Create a new network sender
///
/// # Parameters
/// - `host`: Null-terminated hostname or IP address
/// - `port`: Port number
///
/// # Returns
/// - Pointer to NetworkSender on success
/// - NULL on failure
#[no_mangle]
pub unsafe extern "C" fn seaview_network_create_sender(
    host: *const c_char,
    port: u16,
) -> *mut NetworkSender {
    seaview_network_create_sender_with_config(host, port, seaview_network_default_config())
}

/// Create a new network sender with custom configuration
///
/// # Parameters
/// - `host`: Null-terminated hostname or IP address
/// - `port`: Port number
/// - `config`: Sender configuration
///
/// # Returns
/// - Pointer to NetworkSender on success
/// - NULL on failure
#[no_mangle]
pub unsafe extern "C" fn seaview_network_create_sender_with_config(
    host: *const c_char,
    port: u16,
    config: CSenderConfig,
) -> *mut NetworkSender {
    if host.is_null() {
        error!("Null host pointer provided");
        return ptr::null_mut();
    }

    // Convert C string to Rust string
    let host_str = match CStr::from_ptr(host).to_str() {
        Ok(s) => s,
        Err(e) => {
            error!("Invalid UTF-8 in host string: {}", e);
            return ptr::null_mut();
        }
    };

    // Convert C config to Rust config
    let wire_format = match config.format {
        CWireFormat::Bincode => WireFormat::Bincode,
        #[cfg(feature = "json")]
        CWireFormat::Json => WireFormat::Json,
        #[cfg(not(feature = "json"))]
        CWireFormat::Json => {
            error!("JSON format requested but not compiled with json feature");
            return ptr::null_mut();
        }
    };

    let sender_config = SenderConfig {
        format: wire_format,
        max_message_size: config.max_message_size,
        tcp_nodelay: config.tcp_nodelay != 0,
        send_buffer_size: if config.send_buffer_size > 0 {
            Some(config.send_buffer_size)
        } else {
            None
        },
        connect_timeout: if config.connect_timeout_ms > 0 {
            Some(Duration::from_millis(config.connect_timeout_ms as u64))
        } else {
            None
        },
        write_timeout: if config.write_timeout_ms > 0 {
            Some(Duration::from_millis(config.write_timeout_ms as u64))
        } else {
            None
        },
    };

    let addr = format!("{host_str}:{port}");
    debug!("Creating sender to {}", addr);

    match MeshSender::connect_with_config(&addr, sender_config) {
        Ok(sender) => {
            debug!("Successfully created sender to {}", addr);
            Box::into_raw(Box::new(NetworkSender { sender }))
        }
        Err(e) => {
            error!("Failed to create sender: {}", e);
            ptr::null_mut()
        }
    }
}

/// Send a mesh frame
///
/// # Parameters
/// - `sender`: Sender handle
/// - `mesh`: Mesh frame data
///
/// # Returns
/// - 0 on success
/// - -1 on invalid parameters
/// - -2 on send failure
#[no_mangle]
pub unsafe extern "C" fn seaview_network_send_mesh(
    sender: *mut NetworkSender,
    mesh: *const CMeshFrame,
) -> c_int {
    if sender.is_null() || mesh.is_null() {
        error!("Null pointer passed to send_mesh");
        return -1;
    }

    let sender = &mut (*sender);
    let mesh = &*mesh;

    // Validate mesh data
    if mesh.simulation_id.is_null() {
        error!("Null simulation_id");
        return -1;
    }

    if mesh.vertices.is_null() {
        error!("Null vertices pointer");
        return -1;
    }

    if mesh.vertex_count == 0 || mesh.vertex_count % 3 != 0 {
        error!(
            "Invalid vertex count: {} (must be non-zero and divisible by 3)",
            mesh.vertex_count
        );
        return -1;
    }

    // Convert simulation ID
    let sim_id = match CStr::from_ptr(mesh.simulation_id).to_str() {
        Ok(s) => s.to_string(),
        Err(e) => {
            error!("Invalid UTF-8 in simulation_id: {}", e);
            return -1;
        }
    };

    // Create Rust mesh frame
    let mut rust_mesh = MeshFrame::new(sim_id, mesh.frame_number);
    rust_mesh.timestamp = mesh.timestamp;
    rust_mesh.domain_bounds = DomainBounds::new(mesh.domain_min, mesh.domain_max);

    // Copy vertices
    let vertex_slice = slice::from_raw_parts(mesh.vertices, mesh.vertex_count * 3);
    rust_mesh.vertices = vertex_slice.to_vec();

    // Copy normals if present
    if !mesh.normals.is_null() {
        let normal_slice = slice::from_raw_parts(mesh.normals, mesh.vertex_count * 3);
        rust_mesh.normals = Some(normal_slice.to_vec());
    }

    // Copy indices if present
    if mesh.index_count > 0 && !mesh.indices.is_null() {
        let index_slice = slice::from_raw_parts(mesh.indices, mesh.index_count);
        rust_mesh.indices = Some(index_slice.to_vec());
    }

    // Validate the mesh
    if let Err(e) = rust_mesh.validate() {
        error!("Invalid mesh data: {}", e);
        return -1;
    }

    // Send the mesh
    match sender.sender.send_mesh(&rust_mesh) {
        Ok(()) => {
            debug!(
                "Successfully sent mesh frame {} with {} vertices",
                mesh.frame_number, mesh.vertex_count
            );
            0
        }
        Err(e) => {
            error!("Failed to send mesh: {}", e);
            -2
        }
    }
}

/// Send a heartbeat message
///
/// # Parameters
/// - `sender`: Sender handle
///
/// # Returns
/// - 0 on success
/// - -1 on invalid parameters
/// - -2 on send failure
#[no_mangle]
pub unsafe extern "C" fn seaview_network_send_heartbeat(sender: *mut NetworkSender) -> c_int {
    if sender.is_null() {
        error!("Null sender pointer");
        return -1;
    }

    let sender = &mut (*sender);

    match sender.sender.send_heartbeat() {
        Ok(()) => {
            debug!("Successfully sent heartbeat");
            0
        }
        Err(e) => {
            error!("Failed to send heartbeat: {}", e);
            -2
        }
    }
}

/// Flush any buffered data
///
/// # Parameters
/// - `sender`: Sender handle
///
/// # Returns
/// - 0 on success
/// - -1 on invalid parameters
/// - -2 on flush failure
#[no_mangle]
pub unsafe extern "C" fn seaview_network_flush(sender: *mut NetworkSender) -> c_int {
    if sender.is_null() {
        error!("Null sender pointer");
        return -1;
    }

    let sender = &mut (*sender);

    match sender.sender.flush() {
        Ok(()) => {
            debug!("Successfully flushed sender");
            0
        }
        Err(e) => {
            error!("Failed to flush: {}", e);
            -2
        }
    }
}

/// Get sender statistics
///
/// # Parameters
/// - `sender`: Sender handle
/// - `frames_sent`: Pointer to store frames sent count
/// - `bytes_sent`: Pointer to store bytes sent count
///
/// # Returns
/// - 0 on success
/// - -1 on invalid parameters
#[no_mangle]
pub unsafe extern "C" fn seaview_network_get_stats(
    sender: *mut NetworkSender,
    frames_sent: *mut u64,
    bytes_sent: *mut u64,
) -> c_int {
    if sender.is_null() || frames_sent.is_null() || bytes_sent.is_null() {
        error!("Null pointer passed to get_stats");
        return -1;
    }

    let sender = &(*sender);
    let stats = sender.sender.stats();

    *frames_sent = stats.frames_sent;
    *bytes_sent = stats.bytes_sent;

    0
}

/// Destroy a network sender
///
/// # Parameters
/// - `sender`: Sender handle to destroy
#[no_mangle]
pub unsafe extern "C" fn seaview_network_destroy_sender(sender: *mut NetworkSender) {
    if sender.is_null() {
        return;
    }

    debug!("Destroying network sender");

    // Take ownership and drop
    let sender = Box::from_raw(sender);

    // Try to shutdown gracefully
    match sender.sender.shutdown() {
        Ok(()) => debug!("Sender shutdown successfully"),
        Err(e) => error!("Error during sender shutdown: {}", e),
    }
}

/// Get the last error message
///
/// # Returns
/// - Null-terminated error string
/// - NULL if no error
///
/// Note: The returned string is only valid until the next FFI call
#[no_mangle]
pub extern "C" fn seaview_network_last_error() -> *const c_char {
    // For now, return NULL as we don't have thread-local error storage
    // This could be enhanced in the future
    ptr::null()
}

/// Get the library version string
///
/// # Returns
/// - Null-terminated version string
#[no_mangle]
pub extern "C" fn seaview_network_version() -> *const c_char {
    static VERSION: &[u8] = b"0.1.0\0";
    VERSION.as_ptr() as *const c_char
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = seaview_network_default_config();
        assert_eq!(config.tcp_nodelay, 1);
        assert_eq!(config.max_message_size, 100 * 1024 * 1024);
    }

    #[test]
    fn test_null_safety() {
        // Test null host
        let sender = unsafe { seaview_network_create_sender(ptr::null(), 9999) };
        assert!(sender.is_null());

        // Test null sender in send_mesh
        let result = unsafe { seaview_network_send_mesh(ptr::null_mut(), ptr::null()) };
        assert_eq!(result, -1);

        // Test null mesh
        let result = unsafe { seaview_network_send_heartbeat(ptr::null_mut()) };
        assert_eq!(result, -1);
    }

    #[test]
    fn test_version() {
        let version = seaview_network_version();
        assert!(!version.is_null());

        let version_str = unsafe { CStr::from_ptr(version).to_str().unwrap() };
        assert_eq!(version_str, "0.1.0");
    }
}
