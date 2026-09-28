//! Timed commands waiting for their frame. The queue is sorted by frame and
//! pre-allocated, so pushing and popping never allocate.

use std::collections::VecDeque;

use crate::spec::protocol::Command;

#[derive(Clone, Copy, Debug)]
pub struct Event {
    pub frame: u64,
    pub cmd: Command,
}

pub struct EventQueue {
    q: VecDeque<Event>,
    cap: usize,
    /// Events rejected because the queue was full.
    pub drops: u32,
}

impl EventQueue {
    pub fn with_capacity(cap: usize) -> Self {
        EventQueue { q: VecDeque::with_capacity(cap), cap, drops: 0 }
    }

    /// Insert after any events at the same frame, so order is kept.
    pub fn push(&mut self, ev: Event) {
        if self.q.len() >= self.cap {
            self.drops += 1;
            return;
        }
        let mut i = self.q.len();
        while i > 0 && self.q[i - 1].frame > ev.frame {
            i -= 1;
        }
        self.q.insert(i, ev);
    }

    #[inline]
    pub fn front(&self) -> Option<&Event> {
        self.q.front()
    }

    #[inline]
    pub fn pop_front(&mut self) -> Option<Event> {
        self.q.pop_front()
    }

    #[inline]
    pub fn get(&self, i: usize) -> Option<&Event> {
        self.q.get(i)
    }

    #[inline]
    pub fn remove(&mut self, i: usize) -> Option<Event> {
        self.q.remove(i)
    }

    pub fn clear(&mut self) {
        self.q.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(frame: u64, id: u16) -> Event {
        Event { frame, cmd: Command::SetParam { id, value: 0.0 } }
    }

    #[test]
    fn sorted_and_stable() {
        let mut q = EventQueue::with_capacity(8);
        q.push(ev(30, 1));
        q.push(ev(10, 2));
        q.push(ev(30, 3));
        q.push(ev(0, 4));
        let order: Vec<u16> = std::iter::from_fn(|| q.pop_front())
            .map(|e| match e.cmd {
                Command::SetParam { id, .. } => id,
                _ => 0,
            })
            .collect();
        assert_eq!(order, [4, 2, 1, 3]);
    }

    #[test]
    fn full_queue_drops() {
        let mut q = EventQueue::with_capacity(2);
        for i in 0..5 {
            q.push(ev(i, 0));
        }
        assert_eq!(q.q.len(), 2);
        assert_eq!(q.drops, 3);
    }
}
