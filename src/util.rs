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
	F: Fn() -> Result<R, E>,
{
	let mut attempt: u64 = 0;

	loop {
		attempt += 1;

		let result = closure();
		match result {
			Ok(_) => return result,
			Err(_) => {
				if attempt > 10 {
					return result;
				}

				thread::sleep(time::Duration::from_millis(attempt.pow(2) * 50));
			}
		}
	}
}
