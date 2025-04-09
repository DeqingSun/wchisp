//! USB Transportation.
use std::time::Duration;

use anyhow::Result;
use rusb::{Context, DeviceHandle, UsbContext};

use super::Transport;

const ENDPOINT_OUT: u8 = 0x02;
const ENDPOINT_IN: u8 = 0x82;

const USB_TIMEOUT_MS: u64 = 5000;


pub struct UsbTransport {
    device_handle: DeviceHandle<rusb::Context>,
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    using_ch375_driver: bool,
}




impl UsbTransport {
    pub fn scan_devices() -> Result<usize> {
        let context = Context::new()?;

        let n = context
            .devices()?
            .iter()
            .filter(|device| {
                device
                    .device_descriptor()
                    .map(|desc| {
                        (desc.vendor_id() == 0x4348 || desc.vendor_id() == 0x1a86)
                            && desc.product_id() == 0x55e0
                    })
                    .unwrap_or(false)
            })
            .enumerate()
            .map(|(i, device)| {
                log::debug!("Found WCH ISP USB device #{}: [{:?}]", i, device);
            })
            .count();
        Ok(n)
    }

    pub fn open_nth(nth: usize) -> Result<UsbTransport> {
        log::info!("Opening USB device #{}", nth);

        #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
        {
            log::info!("CH375USBDevice #{}", nth);
            //ch375_driver::CH375USBDevice::open_nth(vid, pid, nth);
        }

        let context = Context::new()?;

        let device = context
            .devices()?
            .iter()
            .filter(|device| {
                device
                    .device_descriptor()
                    .map(|desc| {
                        (desc.vendor_id() == 0x4348 || desc.vendor_id() == 0x1a86)
                            && desc.product_id() == 0x55e0
                    })
                    .unwrap_or(false)
            })
            .nth(nth)
            .ok_or(anyhow::format_err!(
                "No WCH ISP USB device found(4348:55e0 or 1a86:55e0 device not found at index #{})",
                nth
            ))?;
        log::debug!("Found USB Device {:?}", device);

        let device_handle = match device.open() {
            Ok(handle) => handle,
            #[cfg(target_os = "windows")]
            Err(rusb::Error::NotSupported) => {
                log::error!("Failed to open USB device: {:?}", device);
                log::warn!("It's likely no WinUSB/LibUSB drivers installed. Please install it from Zadig. See also: https://zadig.akeo.ie");
                anyhow::bail!("Failed to open USB device on Windows");
            }
            #[cfg(target_os = "linux")]
            Err(rusb::Error::Access) => {
                log::error!("Failed to open USB device: {:?}", device);
                log::warn!("It's likely the udev rules is not installed properly. Please refer to README.md for more details.");
                anyhow::bail!("Failed to open USB device on Linux due to no enough permission");
            }
            Err(e) => {
                log::error!("Failed to open USB device: {}", e);
                anyhow::bail!("Failed to open USB device");
            }
        };

        let config = device.config_descriptor(0)?;

        let mut endpoint_out_found = false;
        let mut endpoint_in_found = false;
        if let Some(intf) = config.interfaces().next() {
            if let Some(desc) = intf.descriptors().next() {
                for endpoint in desc.endpoint_descriptors() {
                    if endpoint.address() == ENDPOINT_OUT {
                        endpoint_out_found = true;
                    }
                    if endpoint.address() == ENDPOINT_IN {
                        endpoint_in_found = true;
                    }
                }
            }
        }

        if !(endpoint_out_found && endpoint_in_found) {
            anyhow::bail!("USB Endpoints not found");
        }

        device_handle.set_active_configuration(1)?;
        let _config = device.active_config_descriptor()?;
        let _descriptor = device.device_descriptor()?;

        device_handle.claim_interface(0)?;

        Ok(UsbTransport { device_handle })
    }

    pub fn open_any() -> Result<UsbTransport> {
        Self::open_nth(0)
    }
}

impl Drop for UsbTransport {
    fn drop(&mut self) {
        // ignore any communication error
        let _ = self.device_handle.release_interface(0);
        // self.device_handle.reset().unwrap();
    }
}

impl Transport for UsbTransport {
    fn send_raw(&mut self, raw: &[u8]) -> Result<()> {
        self.device_handle
            .write_bulk(ENDPOINT_OUT, raw, Duration::from_millis(USB_TIMEOUT_MS))?;
        Ok(())
    }

    fn recv_raw(&mut self, timeout: Duration) -> Result<Vec<u8>> {
        let mut buf = [0u8; 64];
        let nread = self
            .device_handle
            .read_bulk(ENDPOINT_IN, &mut buf, timeout)?;
        Ok(buf[..nread].to_vec())
    }
}

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
pub mod ch375_driver {
    use libloading::os::windows::*;
    use std::fmt;

    use super::*;
    //use crate::Error;

    static mut CH375_DRIVER: Option<Library> = None;

    fn ensure_library_load() -> Result<&'static Library> {
        unsafe {
            if CH375_DRIVER.is_none() {
                CH375_DRIVER = Some(
                    Library::new("WCHLinkDLL.dll")
                        .map_err(|err| "WCHLinkDLL.dll not found")?,
                );
                let lib = CH375_DRIVER.as_ref().unwrap();
                let get_version: Symbol<unsafe extern "stdcall" fn() -> u32> =
                    { lib.get(b"CH375GetVersion").unwrap() };
                let get_driver_version: Symbol<unsafe extern "stdcall" fn() -> u32> =
                    { lib.get(b"CH375GetDrvVersion").unwrap() };

                log::debug!(
                    "DLL version {}, driver version {}",
                    get_version(),
                    get_driver_version()
                );
                Ok(lib)
            } else {
                Ok(CH375_DRIVER.as_ref().unwrap())
            }
        }
    }

    #[allow(non_snake_case, unused)]
    #[derive(Debug)]
    #[repr(packed)]
    pub struct UsbDeviceDescriptor {
        bLength: u8,
        bDescriptorType: u8,
        bcdUSB: u16,
        bDeviceClass: u8,
        bDeviceSubClass: u8,
        bDeviceProtocol: u8,
        bMaxPacketSize0: u8,
        idVendor: u16,
        idProduct: u16,
        bcdDevice: u16,
        iManufacturer: u8,
        iProduct: u8,
        iSerialNumber: u8,
        bNumConfigurations: u8,
    }

    // pub fn list_devices(vid: u16, pid: u16) -> Result<Vec<impl Display>> {
    //     let lib = ensure_library_load()?;
    //     let mut ret: Vec<String> = vec![];

    //     let open_device: Symbol<unsafe extern "stdcall" fn(u32) -> u32> =
    //         unsafe { lib.get(b"CH375OpenDevice").unwrap() };
    //     let close_device: Symbol<unsafe extern "stdcall" fn(u32)> =
    //         unsafe { lib.get(b"CH375CloseDevice").unwrap() };
    //     let get_device_descriptor: Symbol<
    //         unsafe extern "stdcall" fn(u32, *mut UsbDeviceDescriptor, *mut u32) -> bool,
    //     > = unsafe { lib.get(b"CH375GetDeviceDescr").unwrap() };

    //     const INVALID_HANDLE: u32 = 0xffffffff;

    //     for i in 0..8 {
    //         let h = unsafe { open_device(i) };
    //         if h != INVALID_HANDLE {
    //             let mut descr = unsafe { core::mem::zeroed() };
    //             let mut len = core::mem::size_of::<UsbDeviceDescriptor>() as u32;
    //             let _ = unsafe { get_device_descriptor(i, &mut descr, &mut len) };

    //             if descr.idVendor == vid && descr.idProduct == pid {
    //                 ret.push(format!(
    //                     "<WCH-Link#{} WCHLinkDLL device> CH375Driver Device {:04x}:{:04x}",
    //                     i, vid, pid
    //                 ));

    //                 log::debug!("Device #{}: {:04x}:{:04x}", i, vid, pid);
    //             }
    //             unsafe { close_device(i) };
    //         }
    //     }

    //     Ok(ret)
    // }

    /// USB Device implementation provided by CH375 Windows driver
    pub struct CH375USBDevice {
        index: u32,
    }

    impl fmt::Debug for CH375USBDevice {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("USBDevice")
                .field("provider", &"ch375")
                .field("device", &self.index)
                .finish()
        }
    }

    impl USBDeviceBackend for CH375USBDevice {
        fn open_nth(vid: u16, pid: u16, nth: usize) -> Result<Box<dyn USBDeviceBackend>> {
            let lib = ensure_library_load()?;
            /*HANDLE WINAPI CH375OpenDevice( // Open CH375 device, return the handle, invalid if error
            ULONG	iIndex );  */
            let open_device: Symbol<unsafe extern "stdcall" fn(u32) -> u32> =
                unsafe { lib.get(b"CH375OpenDevice").unwrap() };
            /*VOID WINAPI CH375CloseDevice( // Close the CH375 device
            ULONG	iIndex );         // Specify the serial number of the CH375 device */
            let close_device: Symbol<unsafe extern "stdcall" fn(u32)> =
                unsafe { lib.get(b"CH375CloseDevice").unwrap() };
            let get_device_descriptor: Symbol<
                unsafe extern "stdcall" fn(u32, *mut UsbDeviceDescriptor, *mut u32) -> bool,
            > = unsafe { lib.get(b"CH375GetDeviceDescr").unwrap() };

            const INVALID_HANDLE: u32 = 0xffffffff;

            let mut idx = 0;
            for i in 0..8 {
                let h = unsafe { open_device(i) };
                if h != INVALID_HANDLE {
                    let mut descr = unsafe { core::mem::zeroed() };
                    let mut len = core::mem::size_of::<UsbDeviceDescriptor>() as u32;
                    let _ = unsafe { get_device_descriptor(i, &mut descr, &mut len) };

                    if descr.idVendor == vid && descr.idProduct == pid {
                        if idx == nth {
                            log::debug!("Device #{}: {:04x}:{:04x}", i, vid, pid);
                            return Ok(Box::new(CH375USBDevice { index: i }));
                        } else {
                            idx += 1;
                        }
                    }
                    unsafe { close_device(i) };
                }
            }

            return Err("ProbeNotFound");
        }

        // fn read_endpoint(&mut self, ep: u8, buf: &mut [u8]) -> Result<usize> {
        //     let lib = ensure_library_load()?;
        //     /*
        //     BOOL WINAPI CH375ReadEndP( // read data block
        //     ULONG	iIndex,        // Specify the serial number of the CH375 device
        //     ULONG	iPipeNum,      // Endpoint number, valid values are 1 to 8.
        //     PVOID	oBuffer,       // Point to a buffer large enough to hold the read data
        //     PULONG	ioLength);     // Point to the length unit, the length to be read when input, and the actual read length after return
        //      */
        //     let read_end_point: Symbol<
        //         unsafe extern "stdcall" fn(u32, u32, *mut u8, *mut u32) -> bool,
        //     > = unsafe { lib.get(b"CH375ReadEndP").unwrap() };

        //     let mut len = buf.len() as u32;
        //     let ep = (ep & 0x7f) as u32;

        //     let ret = unsafe { read_end_point(self.index, ep, buf.as_mut_ptr(), &mut len) };

        //     if ret {
        //         Ok(len as usize)
        //     } else {
        //         Err(Error::Driver)
        //     }
        // }

        // fn write_endpoint(&mut self, ep: u8, buf: &[u8]) -> Result<()> {
        //     let lib = ensure_library_load()?;
        //     /*
        //         BOOL WINAPI CH375WriteEndP( // write out data block
        //     ULONG	iIndex,         // Specify the serial number of the CH375 device
        //     ULONG	iPipeNum,       // Endpoint number, valid values are 1 to 8.
        //     PVOID	iBuffer,        // Point to a buffer where the data to be written is placed
        //     PULONG	ioLength);      // Point to the length unit, the length to be written out when input, and the length actually written out after returnF */
        //     let write_end_point: Symbol<
        //         unsafe extern "stdcall" fn(u32, u32, *mut u8, *mut u32) -> bool,
        //     > = unsafe { lib.get(b"CH375WriteEndP").unwrap() };

        //     let mut len = buf.len() as u32;
        //     let ret = unsafe {
        //         write_end_point(self.index, ep as u32, buf.as_ptr() as *mut u8, &mut len)
        //     };
        //     if ret {
        //         Ok(())
        //     } else {
        //         Err(Error::Driver)
        //     }
        // }

        fn set_timeout(&mut self, timeout: Duration) {
            let lib = ensure_library_load().unwrap();

            let set_timeout_ex: Symbol<
                unsafe extern "stdcall" fn(u32, u32, u32, u32, u32) -> bool,
            > = unsafe { lib.get(b"CH375SetTimeoutEx").unwrap() };

            let ds = timeout.as_millis() as u32;

            unsafe {
                set_timeout_ex(self.index, ds, ds, ds, ds);
            }
        }
    }

    impl Drop for CH375USBDevice {
        fn drop(&mut self) {
            if let Ok(lib) = ensure_library_load() {
                let close_device: Symbol<unsafe extern "stdcall" fn(u32)> =
                    unsafe { lib.get(b"CH375CloseDevice").unwrap() };
                unsafe {
                    close_device(self.index);
                }
            }
        }
    }
}
