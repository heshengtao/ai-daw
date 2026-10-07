use rtrb::{Consumer, Producer, RingBuffer};

use crate::transport::TransportCommand;

/// 控制侧发送端（非实时线程使用）。
pub struct CommandSender {
    producer: Producer<TransportCommand>,
}

/// 音频侧接收端（实时线程使用，`drain` 为 lock-free / wait-free）。
pub struct CommandReceiver {
    consumer: Consumer<TransportCommand>,
}

/// 建立一对无锁 SPSC 命令通道。
pub fn command_queue(capacity: usize) -> (CommandSender, CommandReceiver) {
    let (producer, consumer) = RingBuffer::new(capacity);
    (CommandSender { producer }, CommandReceiver { consumer })
}

impl CommandSender {
    /// 非阻塞发送；队列满时返回 false（命令被丢弃，永不阻塞）。
    pub fn send(&mut self, command: TransportCommand) -> bool {
        self.producer.push(command).is_ok()
    }
}

impl CommandReceiver {
    /// 取出当前所有命令并按顺序回调。实时安全。
    pub fn drain(&mut self, mut apply: impl FnMut(TransportCommand)) {
        while let Ok(command) = self.consumer.pop() {
            apply(command);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::TransportCommand;

    #[test]
    fn send_then_drain_in_order() {
        let (mut tx, mut rx) = command_queue(8);
        assert!(tx.send(TransportCommand::Play));
        assert!(tx.send(TransportCommand::Seek { frame: 1234 }));
        let mut seen = Vec::new();
        rx.drain(|c| seen.push(c));
        assert_eq!(seen, vec![TransportCommand::Play, TransportCommand::Seek { frame: 1234 }]);
        // queue is now empty
        let mut empty = Vec::new();
        rx.drain(|c| empty.push(c));
        assert!(empty.is_empty());
    }

    #[test]
    fn full_queue_reports_failure_without_blocking() {
        let (mut tx, _rx) = command_queue(1);
        assert!(tx.send(TransportCommand::Play));
        assert!(!tx.send(TransportCommand::Pause));
    }
}
