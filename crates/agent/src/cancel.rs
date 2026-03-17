use std::time::Duration;

use astrcode_core::CancelToken;

pub async fn cancelled(cancel: CancelToken) {
    while !cancel.is_cancelled() {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}
