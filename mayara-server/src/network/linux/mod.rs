pub fn is_wireless_interface(interface_name: &str) -> bool {
    use libc::{c_void, ifreq, ioctl, strncpy, Ioctl, AF_INET};
    use std::ffi::CString;

    const SIOCGIWNAME: Ioctl = 0x8B01; // Wireless Extensions request to get interface name

    // Open a socket for ioctl operations
    let socket_fd = unsafe { libc::socket(AF_INET, libc::SOCK_DGRAM, 0) };
    if socket_fd < 0 {
        return false;
    }

    // Prepare the interface request structure
    let mut ifr = unsafe { std::mem::zeroed::<ifreq>() };
    let iface_cstring = CString::new(interface_name).expect("Invalid interface name");
    unsafe {
        strncpy(
            ifr.ifr_name.as_mut_ptr(),
            iface_cstring.as_ptr(),
            ifr.ifr_name.len(),
        );
    }

    // Perform the ioctl call
    let res = unsafe { ioctl(socket_fd, SIOCGIWNAME, &mut ifr as *mut _ as *mut c_void) };

    // Close the socket
    unsafe { libc::close(socket_fd) };

    match res {
        0 => true, // The interface supports wireless extensions
        _ => false,
    }
}
