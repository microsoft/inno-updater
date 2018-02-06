/*-----------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See LICENSE in the project root for license information.
 *----------------------------------------------------------------------------------------*/

use std::{thread, time};

/**
 * Quadratic backoff retry mechanism
 */
pub fn retry<F, R, E>(closure: F) -> Result<R, E>
where
	F: Fn(u32) -> Result<R, E>,
{
	let mut attempt: u32 = 0;

	loop {
		attempt += 1;

		let result = closure(attempt);
		match result {
			Ok(_) => return result,
			Err(_) => {
				if attempt > 10 {
					return result;
				}

				thread::sleep(time::Duration::from_millis((attempt.pow(2) * 50) as u64));
			}
		}
	}
}
