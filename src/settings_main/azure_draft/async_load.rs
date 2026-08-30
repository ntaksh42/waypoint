//! `AzureProjectPicker` の各非同期読み込みが繰り返す
//! 「`Receiver` を保持し、描画フレームごとに `try_recv` して結果を反映する」
//! という定型を 1 つの型にまとめる。
//!
//! egui のイベントループは受信スレッドからは起こせず、`poll_load` が
//! 描画のたびに `try_recv` するしかない。そのため loader は必ず
//! `AzureProjectPicker::is_loading_anything` から辿れる場所に置く必要が
//! あり、ここへ切り出しても「新しい loader を増やしたら
//! `is_loading_anything` に足す」という規律は呼び出し側に残る。

use std::sync::mpsc::{Receiver, TryRecvError};

/// 進行中の非同期読み込み 1 件。`poll` が `Some` を返したら loader は
/// 消費済みで、以後は再び `start` するまでアイドル状態になる。
pub(super) struct AsyncLoad<T> {
    receiver: Option<Receiver<T>>,
}

impl<T> Default for AsyncLoad<T> {
    fn default() -> Self {
        Self { receiver: None }
    }
}

impl<T> AsyncLoad<T> {
    pub(super) fn start(&mut self, receiver: Receiver<T>) {
        self.receiver = Some(receiver);
    }

    /// `None`: loader が無い、またはまだ結果が届いていない (呼び出し側は
    /// 何もせず抜ける)。`Some(Ok(value))`: 結果が届いた。
    /// `Some(Err(()))`: 送信側が落ちた (ワーカースレッドがパニックした等)。
    /// いずれの `Some` でも loader は消費済み。
    pub(super) fn poll(&mut self) -> Option<Result<T, ()>> {
        let result = match self.receiver.as_ref()?.try_recv() {
            Ok(value) => Ok(value),
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => Err(()),
        };
        self.receiver = None;
        Some(result)
    }
}
