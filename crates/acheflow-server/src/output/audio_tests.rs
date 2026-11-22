use super::*;
use tokio::io::{AsyncReadExt, DuplexStream, duplex};

const BLOCK_BYTES: usize = PCM_BLOCK_BYTES;

fn block(sequence: u64, tag: u8) -> Arc<PcmBlock> {
    Arc::new(PcmBlock {
        sequence,
        scheduled: tokio::time::Instant::now(),
        bytes: vec![tag; BLOCK_BYTES],
    })
}

async fn expect_bytes(reader: &mut DuplexStream, count: usize, value: u8) {
    let mut bytes = vec![0; count];
    tokio::time::timeout(Duration::from_secs(2), reader.read_exact(&mut bytes))
        .await
        .expect("audio writer stalled")
        .expect("audio writer closed");
    assert!(bytes.iter().all(|&byte| byte == value));
}

#[tokio::test]
async fn short_lag_preserves_pcm_length_while_another_destination_keeps_writing() {
    let (slow_tx, slow_rx) = broadcast::channel(8);
    let (healthy_tx, healthy_rx) = broadcast::channel(8);
    let (slow_writer, mut slow_reader) = duplex(BLOCK_BYTES / 2);
    let (healthy_writer, mut healthy_reader) = duplex(BLOCK_BYTES);
    let slow_cancel = CancellationToken::new();
    let healthy_cancel = CancellationToken::new();
    let slow = tokio::spawn({
        let cancel = slow_cancel.clone();
        async move { write_audio(slow_writer, slow_rx, &cancel, None, None).await }
    });
    let healthy = tokio::spawn({
        let cancel = healthy_cancel.clone();
        async move { write_audio(healthy_writer, healthy_rx, &cancel, None, None).await }
    });

    slow_tx.send(block(1, 1)).unwrap();
    expect_bytes(&mut slow_reader, 1, 1).await;
    for tag in 2..=13 {
        let block = block(u64::from(tag), tag);
        slow_tx.send(block.clone()).unwrap();
        healthy_tx.send(block).unwrap();
        expect_bytes(&mut healthy_reader, BLOCK_BYTES, tag).await;
    }
    expect_bytes(&mut slow_reader, BLOCK_BYTES - 1, 1).await;
    expect_bytes(&mut slow_reader, 4 * BLOCK_BYTES, 0).await;
    for tag in 6..=13 {
        expect_bytes(&mut slow_reader, BLOCK_BYTES, tag).await;
    }
    slow_tx.send(block(14, 14)).unwrap();
    expect_bytes(&mut slow_reader, BLOCK_BYTES, 14).await;
    slow_cancel.cancel();
    healthy_cancel.cancel();
    assert!(slow.await.unwrap().is_ok());
    assert!(healthy.await.unwrap().is_ok());
}

#[tokio::test]
async fn lag_beyond_the_recovery_budget_has_its_own_error() {
    let (tx, rx) = broadcast::channel(8);
    for sequence in 0..17 {
        tx.send(block(sequence, 1)).unwrap();
    }
    let (writer, _reader) = duplex(BLOCK_BYTES);
    let error = write_audio(writer, rx, &CancellationToken::new(), None, None)
        .await
        .unwrap_err();
    assert!(error.contains("9 audio blocks lost"), "{error}");
}

#[tokio::test]
async fn repeated_small_gaps_cannot_exceed_the_budget_before_catching_up() {
    let (tx, rx) = broadcast::channel(8);
    for sequence in 0..13 {
        tx.send(block(sequence, 1)).unwrap();
    }
    let (writer, mut reader) = duplex(BLOCK_BYTES);
    let task = tokio::spawn(async move {
        write_audio(writer, rx, &CancellationToken::new(), None, None).await
    });
    // The receiver is writing five missing blocks of silence and has eight
    // one retained block already in hand. Drop four additional blocks before the write finishes.
    expect_bytes(&mut reader, 1, 0).await;
    for sequence in 13..18 {
        tx.send(block(sequence, 2)).unwrap();
    }
    expect_bytes(&mut reader, 5 * BLOCK_BYTES - 1, 0).await;
    expect_bytes(&mut reader, BLOCK_BYTES, 1).await;
    let error = tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .expect("audio writer ignored repeated lag")
        .unwrap()
        .unwrap_err();
    assert!(error.contains("9 audio blocks lost"), "{error}");
}

#[tokio::test]
async fn channel_close_and_socket_error_have_distinct_failures() {
    let (tx, rx) = broadcast::channel(8);
    drop(tx);
    let (writer, _reader) = duplex(BLOCK_BYTES);
    let error = write_audio(writer, rx, &CancellationToken::new(), None, None)
        .await
        .unwrap_err();
    assert_eq!(error, "Audio mixer channel closed");

    let (tx, rx) = broadcast::channel(8);
    tx.send(block(1, 1)).unwrap();
    let (writer, reader) = duplex(BLOCK_BYTES);
    drop(reader);
    let error = write_audio(writer, rx, &CancellationToken::new(), None, None)
        .await
        .unwrap_err();
    assert!(
        error.starts_with("Encoder audio socket write failed"),
        "{error}"
    );
}

#[tokio::test]
async fn cancellation_interrupts_a_blocked_audio_write() {
    let (tx, rx) = broadcast::channel(8);
    let (writer, mut reader) = duplex(BLOCK_BYTES / 2);
    let cancel = CancellationToken::new();
    let task = tokio::spawn({
        let cancel = cancel.clone();
        async move { write_audio(writer, rx, &cancel, None, None).await }
    });
    tx.send(block(1, 1)).unwrap();
    expect_bytes(&mut reader, 1, 1).await;
    cancel.cancel();
    assert!(
        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .expect("cancellation did not interrupt the blocked writer")
            .unwrap()
            .is_ok()
    );
}

#[tokio::test]
async fn mixer_tick_gaps_preserve_pcm_time_without_a_receiver_overrun() {
    let (tx, rx) = broadcast::channel(8);
    let (writer, mut reader) = duplex(BLOCK_BYTES);
    let cancel = CancellationToken::new();
    let task = tokio::spawn({
        let cancel = cancel.clone();
        async move { write_audio(writer, rx, &cancel, None, None).await }
    });
    tx.send(block(3, 1)).unwrap();
    tx.send(block(6, 2)).unwrap();
    expect_bytes(&mut reader, BLOCK_BYTES, 1).await;
    expect_bytes(&mut reader, 2 * BLOCK_BYTES, 0).await;
    expect_bytes(&mut reader, BLOCK_BYTES, 2).await;
    cancel.cancel();
    assert!(task.await.unwrap().is_ok());
}

#[tokio::test]
async fn prefetched_epoch_keeps_the_first_pcm_block_at_zero() {
    let (tx, rx) = broadcast::channel(8);
    let (writer, mut reader) = duplex(BLOCK_BYTES);
    let cancel = CancellationToken::new();
    let task = tokio::spawn({
        let cancel = cancel.clone();
        async move { write_audio(writer, rx, &cancel, Some(block(20, 1)), None).await }
    });
    tx.send(block(22, 2)).unwrap();
    expect_bytes(&mut reader, BLOCK_BYTES, 1).await;
    expect_bytes(&mut reader, BLOCK_BYTES, 0).await;
    expect_bytes(&mut reader, BLOCK_BYTES, 2).await;
    cancel.cancel();
    assert!(task.await.unwrap().is_ok());
}
