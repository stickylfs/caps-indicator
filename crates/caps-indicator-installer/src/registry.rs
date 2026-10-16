use windows::core::PCWSTR;
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegDeleteValueW, RegOpenKeyExW, RegSetValueExW,
    HKEY, KEY_SET_VALUE, KEY_WRITE, REG_DWORD, REG_OPTION_NON_VOLATILE, REG_SZ,
};

pub fn set_reg_string(
    hkey_root: HKEY,
    subkey: &str,
    name: Option<&str>,
    value: &str,
) -> Result<(), u32> {
    let wide_subkey: Vec<u16> = subkey.encode_utf16().chain(std::iter::once(0)).collect();
    let mut hkey = HKEY::default();

    unsafe {
        let res = RegCreateKeyExW(
            hkey_root,
            PCWSTR(wide_subkey.as_ptr()),
            0,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut hkey,
            None,
        );

        if res != ERROR_SUCCESS {
            return Err(res.0);
        }

        let wide_name: Option<Vec<u16>> =
            name.map(|n| n.encode_utf16().chain(std::iter::once(0)).collect());
        let wide_val: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();

        let val_bytes = wide_val.len() * std::mem::size_of::<u16>();
        let name_ptr = match wide_name {
            Some(ref w) => PCWSTR(w.as_ptr()),
            None => PCWSTR::null(),
        };

        let set_res = RegSetValueExW(
            hkey,
            name_ptr,
            0,
            REG_SZ,
            Some(std::slice::from_raw_parts(
                wide_val.as_ptr() as *const u8,
                val_bytes,
            )),
        );

        let _ = RegCloseKey(hkey);

        if set_res != ERROR_SUCCESS {
            return Err(set_res.0);
        }
    }

    Ok(())
}

pub fn set_reg_dword(hkey_root: HKEY, subkey: &str, name: &str, value: u32) -> Result<(), u32> {
    let wide_subkey: Vec<u16> = subkey.encode_utf16().chain(std::iter::once(0)).collect();
    let mut hkey = HKEY::default();

    unsafe {
        let res = RegCreateKeyExW(
            hkey_root,
            PCWSTR(wide_subkey.as_ptr()),
            0,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut hkey,
            None,
        );

        if res != ERROR_SUCCESS {
            return Err(res.0);
        }

        let wide_name: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        let bytes = value.to_ne_bytes();

        let set_res = RegSetValueExW(hkey, PCWSTR(wide_name.as_ptr()), 0, REG_DWORD, Some(&bytes));

        let _ = RegCloseKey(hkey);

        if set_res != ERROR_SUCCESS {
            return Err(set_res.0);
        }
    }

    Ok(())
}

pub fn delete_reg_tree(hkey_root: HKEY, subkey: &str) -> Result<(), u32> {
    let wide_subkey: Vec<u16> = subkey.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let res = RegDeleteTreeW(hkey_root, PCWSTR(wide_subkey.as_ptr()));
        if res == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(res.0)
        }
    }
}

pub fn delete_reg_value(hkey_root: HKEY, subkey: &str, name: &str) -> Result<(), u32> {
    let wide_subkey: Vec<u16> = subkey.encode_utf16().chain(std::iter::once(0)).collect();
    let mut hkey = HKEY::default();

    unsafe {
        let open_res = RegOpenKeyExW(
            hkey_root,
            PCWSTR(wide_subkey.as_ptr()),
            0,
            KEY_SET_VALUE,
            &mut hkey,
        );
        if open_res != ERROR_SUCCESS {
            return Err(open_res.0);
        }

        let wide_name: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        let del_res = RegDeleteValueW(hkey, PCWSTR(wide_name.as_ptr()));
        let _ = RegCloseKey(hkey);

        if del_res == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(del_res.0)
        }
    }
}

pub fn get_reg_string(hkey_root: HKEY, subkey: &str, name: &str) -> Result<String, u32> {
    use windows::Win32::System::Registry::{RegGetValueW, RRF_RT_REG_SZ};
    let wide_subkey: Vec<u16> = subkey.encode_utf16().chain(std::iter::once(0)).collect();
    let wide_name: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    let mut buf = [0u16; 512];
    let mut cb_data = (buf.len() * std::mem::size_of::<u16>()) as u32;

    unsafe {
        let res = RegGetValueW(
            hkey_root,
            PCWSTR(wide_subkey.as_ptr()),
            PCWSTR(wide_name.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr() as *mut _),
            Some(&mut cb_data),
        );
        if res == ERROR_SUCCESS {
            let len = (cb_data as usize / std::mem::size_of::<u16>()).saturating_sub(1);
            let s = String::from_utf16_lossy(&buf[..len]);
            Ok(s)
        } else {
            Err(res.0)
        }
    }
}
