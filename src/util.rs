/*-----------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See LICENSE in the project root for license information.
 *----------------------------------------------------------------------------------------*/

use gui;
use std::{error, ptr, thread, time};
use strings::from_utf16;

/**
 * Quadratic backoff retry mechanism.
 *
 * Use `max_attempts` to control how long it should retry for:
 * 	- 11 (default): 19s
 *  - 16: ~1 minute
 *  - 20: ~2 minutes
 *  - 23: ~3 minutes
 *  - 25: ~4 minutes
 *  - 27: ~5 minutes
 */
pub fn retry<F, R, T>(task: &str, closure: F, max_attempts: T) -> Result<R, Box<error::Error>>
where
	F: Fn(u32) -> Result<R, Box<error::Error>>,
	T: Into<Option<u32>>,
{
	let mut attempt: u32 = 0;
	let max_attempts = max_attempts.into().unwrap_or(11);

	loop {
		attempt += 1;

		let result = closure(attempt);
		match result {
			Ok(_) => return result,
			Err(err) => {
				if attempt >= max_attempts {
					let msg = format!("There was an error while {}:\n\n{}\n\nPlease verify there are no VS Code processes still executing.", task, err);
					let mb_result =
						gui::message_box(&msg, "VS Code", gui::MessageBoxType::RetryCancel);

					match mb_result {
						gui::MessageBoxResult::Retry => {
							attempt = 0;
						}
						_ => {
							return Err(err);
						}
					}
				}

				thread::sleep(time::Duration::from_millis((attempt.pow(2) * 50) as u64));
			}
		}
	}
}

pub fn get_last_error_message() -> Result<String, Box<error::Error>> {
	use winapi::um::errhandlingapi::GetLastError;
	use winapi::um::winbase::{
		FormatMessageW, FORMAT_MESSAGE_FROM_SYSTEM, FORMAT_MESSAGE_IGNORE_INSERTS,
	};

	let mut error_message = [0u16; 32000];
	let error_message_len: usize;

	unsafe {
		error_message_len = FormatMessageW(
			FORMAT_MESSAGE_FROM_SYSTEM | FORMAT_MESSAGE_IGNORE_INSERTS,
			ptr::null_mut(),
			GetLastError(),
			0,
			error_message.as_mut_ptr(),
			32000,
			ptr::null_mut(),
		) as usize;
	}

	Ok(match error_message_len {
		0 => String::from("unknown error"),
		_ => from_utf16(&error_message[0..error_message_len])?,
	})
}
