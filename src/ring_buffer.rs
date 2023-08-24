pub struct RingBuffer<T> {
    buffer: Vec<Option<T>>,
    capacity: usize,
    write_idx: usize,
    loop_num: usize,
}

impl<T> RingBuffer<T> {
    pub fn new(capacity: usize) -> Self {
        RingBuffer {
            buffer: (0..capacity).map(|_| None).collect(),
            capacity,
            write_idx: 0,
            loop_num: 0,
        }
    }

    pub fn push(&mut self, item: T) {
        self.buffer[self.write_idx] = Some(item);
        let mut write_idx = self.write_idx + 1;
        if write_idx == self.capacity {
            self.loop_num += 1;
            write_idx = 0;
        }
        self.write_idx = write_idx;
    }

    pub fn iter(&self) -> RingBufferIter<'_, T> {
        RingBufferIter {
            ring_buffer: self,
            index: if self.loop_num > 0 { self.write_idx } else { 0 },
            remaining: self.capacity,
        }
    }
    pub fn get_next_number(&self) -> usize {
        (self.loop_num * self.capacity) + self.write_idx
    }
    pub fn get_base_index(&self) -> usize {
        if self.loop_num == 0 {
            return 0;
        }
        self.get_next_number() - self.capacity
    }
}

pub struct RingBufferIter<'a, T> {
    ring_buffer: &'a RingBuffer<T>,
    index: usize,
    remaining: usize,
}

impl<'a, T> Iterator for RingBufferIter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining > 0 {
            let item = &self.ring_buffer.buffer[self.index];
            self.index = (self.index + 1) % self.ring_buffer.capacity;
            self.remaining -= 1;
            if item.is_some() {
                return item.as_ref();
            }
        }
        None
    }
}

#[test]
fn buffer_test() {
    let mut ring_buffer = RingBuffer::new(3);
    ring_buffer.push(0);
    assert_eq!(ring_buffer.iter().map(|x| x.clone()).collect::<Vec<i32>>(), vec![0]);
    ring_buffer.push(1);
    assert_eq!(ring_buffer.iter().map(|x| x.clone()).collect::<Vec<i32>>(), vec![0, 1]);
    ring_buffer.push(2);
    assert_eq!(ring_buffer.iter().map(|x| x.clone()).collect::<Vec<i32>>(), vec![0, 1, 2]);
    ring_buffer.push(3);
    assert_eq!(ring_buffer.iter().map(|x| x.clone()).collect::<Vec<i32>>(), vec![1, 2, 3]);
    ring_buffer.push(4);
    assert_eq!(ring_buffer.iter().map(|x| x.clone()).collect::<Vec<i32>>(), vec![2, 3, 4]);
}

#[test]
fn get_base_index_test() {
    let mut ring_buffer = RingBuffer::new(3);
    for i in 0..10 {
        ring_buffer.push(i);
        let base_index = ring_buffer.get_base_index();
        let last_number = ring_buffer.iter().enumerate().map(|(id, _)| id + base_index).last().unwrap();
        assert_eq!(i, last_number);
    }
}

