pub fn is_wireless_interface(interface_name: &str) -> bool {
    use std::ptr::null_mut;
    use windows::Win32::NetworkManagement::WiFi::{
        WlanCloseHandle, WlanEnumInterfaces, WlanFreeMemory, WlanOpenHandle,
        WLAN_INTERFACE_INFO_LIST,
    };

    unsafe {
        // Open WLAN handle
        let mut client_handle: HANDLE = Default::default();
        let mut negotiated_version = 0;
        let wlan_result = WlanOpenHandle(2, None, &mut negotiated_version, &mut client_handle);

        if wlan_result == ERROR_SERVICE_NOT_ACTIVE.0 {
            return false;
        }
        if wlan_result != 0 {
            panic!("WlanOpenHandle failed with error: {}", wlan_result);
        }

        let mut interface_list: *mut WLAN_INTERFACE_INFO_LIST = null_mut();
        let wlan_enum_result = WlanEnumInterfaces(client_handle, None, &mut interface_list);

        if wlan_enum_result != 0 {
            WlanCloseHandle(client_handle, None);
            panic!("WlanEnumInterfaces failed with error: {}", wlan_enum_result);
        }

        let interfaces = &*interface_list;

        // Check each WLAN interface
        for i in 0..interfaces.dwNumberOfItems {
            let wlan_interface = &interfaces.InterfaceInfo[i as usize];
            let wlan_interface_name =
                String::from_utf16_lossy(&wlan_interface.strInterfaceDescription);
            if wlan_interface_name.trim() == interface_name.trim() {
                WlanFreeMemory(interface_list as _);
                WlanCloseHandle(client_handle, None);
                return true;
            }
        }

        WlanFreeMemory(interface_list as _);
        WlanCloseHandle(client_handle, None);
    }

    false
}
