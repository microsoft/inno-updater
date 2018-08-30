/*-----------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See LICENSE in the project root for license information.
 *----------------------------------------------------------------------------------------*/

use std::path::Path;
use std::{error, io, ptr};
use strings::to_u16s;
use util;
use winapi::um::winnt::HANDLE;

pub struct FileHandle(HANDLE);

impl FileHandle {
    pub fn new(path: &Path) -> Result<FileHandle, Box<error::Error>> {
        use winapi::um::fileapi::CreateFileW;
        use winapi::um::fileapi::OPEN_EXISTING;
        use winapi::um::handleapi::INVALID_HANDLE_VALUE;
        use winapi::um::winnt::DELETE;
        use winapi::um::winnt::FILE_ATTRIBUTE_NORMAL;

        unsafe {
            let handle = CreateFileW(
                to_u16s(path.as_os_str()).as_ptr(),
                DELETE,
                0,
                ptr::null_mut(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                ptr::null_mut(),
            );

            if handle == INVALID_HANDLE_VALUE {
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    format!(
                        "Failed to create file handle: {}",
                        util::get_last_error_message()?
                    ),
                ).into());
            }

            Ok(FileHandle(handle))
        }
    }

    pub fn mark_for_deletion(&self) -> Result<(), Box<error::Error>> {
        use std::mem;
        use winapi::shared::minwindef::{DWORD, FALSE, LPVOID, TRUE};
        use winapi::um::fileapi::SetFileInformationByHandle;
        use winapi::um::fileapi::FILE_DISPOSITION_INFO;
        use winapi::um::minwinbase::FileDispositionInfo;

        unsafe {
            let mut info = FILE_DISPOSITION_INFO { DeleteFile: TRUE };
            let result = SetFileInformationByHandle(
                self.0,
                FileDispositionInfo,
                &mut info as *mut _ as LPVOID,
                mem::size_of::<FILE_DISPOSITION_INFO>() as DWORD,
            );

            if result == FALSE {
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    format!(
                        "Failed to mark file for deletion: {}",
                        util::get_last_error_message()?
                    ),
                ).into());
            }
        }

        Ok(())
    }

    pub fn close(&self) -> Result<(), Box<error::Error>> {
        use winapi::shared::minwindef::FALSE;
        use winapi::um::handleapi::CloseHandle;

        unsafe {
            if CloseHandle(self.0) == FALSE {
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    format!(
                        "Failed to close file handle: {}",
                        util::get_last_error_message()?
                    ),
                ).into());
            }
        }

        Ok(())
    }
}
