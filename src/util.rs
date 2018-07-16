/*-----------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See LICENSE in the project root for license information.
 *----------------------------------------------------------------------------------------*/

use std::{thread, time};

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
pub fn retry<F, R, E, T>(closure: F, max_attempts: T) -> Result<R, E>
where
	F: Fn(u32) -> Result<R, E>,
	T: Into<Option<u32>>,
{
	let mut attempt: u32 = 0;
	let max_attempts = max_attempts.into().unwrap_or(11);

	loop {
		attempt += 1;

		let result = closure(attempt);
		match result {
			Ok(_) => return result,
			Err(_) => {
				if attempt >= max_attempts {
					return result;
				}

				thread::sleep(time::Duration::from_millis((attempt.pow(2) * 50) as u64));
			}
		}
	}
}
