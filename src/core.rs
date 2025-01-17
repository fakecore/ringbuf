//! The implementation of mpmc-ringbuf.
//!
//! fixed mode(default): buffer block is fixed, can set it with fn `set_fixed()`
//!
//! dynamic mode: buffer block is fixed, can set it with fn `set_dynamic()`
//!
use crate::core::BufferCacheMode::{Dynamic, Fixed};
use core::panicking::panic;
use std::borrow::BorrowMut;
use std::cell::RefCell;
use std::collections::HashMap;
use std::os::unix::raw::mode_t;
use std::rc::Rc;

/// main struct for controlling buffer blocks
/// Users should crate a new buffer block with fn `add_producer`, and explicitly delete the buffer
/// blocks with fn `delete_consumer`
/// If users want to add a producer and produce some data, make sure you call fn `add_producer` ahead,
/// otherwise there are none data block for storing the data.
pub struct MsgQueue<T> {
    inner: Rc<RefCell<MsgQueueInner<T>>>,
    serial_no: u64,
    running: bool,
}

unsafe impl<T> Sync for MsgQueue<T> {}
unsafe impl<T> Send for MsgQueue<T> {}

/// @TODO MsgQueue should manage the exist blocks for querying
impl<T> MsgQueue<T>
where
    T: Default + Clone,
{
    pub fn new() -> MsgQueue<T> {
        let inner = Rc::new(RefCell::new(MsgQueueInner {
            buf: HashMap::new(),
            mode: None,
            buf_size: 0,
            block_length: 0,
        }));

        MsgQueue {
            inner: inner.clone(),
            serial_no: 0,
            running: false,
        }
    }

    /// only can call before using `get_consumer` and `add_consumer`
    pub fn set_dynamic(&mut self, block_length: u64) {
        if self.running == true {
            panic!("Rb is running, please config before running");
            return;
        }
        (*self.inner).borrow_mut().set_dynamic(block_length);
    }

    pub fn set_fixed(&mut self, block_length: u64, buf_size: u64) {
        if self.running == true {
            panic!("Rb is running, please config before running");
            return;
        }
        (*self.inner).borrow_mut().set_fixed(block_length, buf_size);
    }

    pub fn add_producer(&mut self) -> MsgQueueWriter<T> {
        MsgQueueWriter {
            inner: self.inner.clone(),
        }
    }

    /// get_consumer won't panic even buffer block doesn't exist,
    /// system will check the matched block and create it when it doesn't exist.
    pub fn get_consumer(&mut self, id: u64) -> MsgQueueReader<T> {
        if self.running == false {
            self.running = true;
        }

        let mut buf = (*self.inner).borrow_mut();
        buf.add_buffer_cache(id);
        MsgQueueReader {
            id,
            inner: self.inner.clone(),
        }
    }

    //
    pub fn add_consumer(&mut self) -> MsgQueueReader<T> {
        if self.running == false {
            self.running = true;
        }

        let id = self.serial_no;
        self.serial_no += 1;
        let mut buf = (*self.inner).borrow_mut();
        buf.add_buffer_cache(id);
        MsgQueueReader {
            id,
            inner: self.inner.clone(),
        }
    }

    pub fn get_consumer_count(&self) -> u64 {
        (*self.inner).borrow().buf.len() as u64
    }

    pub fn delete_consumer(&mut self, id: u64) {
        (*self.inner).borrow_mut().delete_buffer_cache(id)
    }

    pub fn is_running(&self) -> bool {
        self.running
    }
}

struct MsgQueueInner<T> {
    buf: HashMap<u64, BufferCache<T>>,
    mode: Option<BufferCacheMode>,
    buf_size: u64,
    block_length: u64,
}

impl<T> MsgQueueInner<T>
where
    T: Default + Clone,
{
    pub fn add_buffer_cache(&mut self, id: u64) {
        if !self.buf.contains_key(&id) {
            let mut buffer_cache = BufferCache::new();
            match self.mode {
                None => {}
                Some(mode) => {
                    if mode == Fixed {
                        buffer_cache.set_fixed_mode(self.buf_size, self.block_length);
                    } else if mode == Dynamic {
                        buffer_cache.set_dynamic_mode(self.block_length);
                    }
                }
            }
            self.buf.insert(id, buffer_cache);
        }
    }

    pub fn get_buffer_cache(&mut self, id: u64) -> Option<&mut BufferCache<T>> {
        if !self.buf.contains_key(&id) {
            let mut buffer_cache = BufferCache::new();
            match self.mode {
                None => {}
                Some(mode) => {
                    if mode == Fixed {
                        buffer_cache.set_fixed_mode(self.buf_size, self.block_length);
                    } else if mode == Dynamic {
                        buffer_cache.set_dynamic_mode(self.block_length);
                    }
                }
            }
            self.buf.insert(id, buffer_cache);
        }
        self.buf.get_mut(&id)
    }

    pub fn delete_buffer_cache(&mut self, id: u64) {
        if !self.buf.contains_key(&id) {
            self.buf.remove(&id);
        }
    }

    pub fn set_dynamic(&mut self, block_length: u64) {
        self.mode = Some(Dynamic);
        self.block_length = block_length;
    }

    pub fn set_fixed(&mut self, block_length: u64, buf_size: u64) {
        self.mode = Some(Fixed);
        self.block_length = block_length;
        self.buf_size = buf_size;
    }
}

/// for data reader
pub struct MsgQueueReader<T> {
    id: u64,
    inner: Rc<RefCell<MsgQueueInner<T>>>,
}

/// for data writer
pub struct MsgQueueWriter<T> {
    inner: Rc<RefCell<MsgQueueInner<T>>>,
}

impl<T> MsgQueueReader<T>
where
    T: Default + Clone,
{
    pub fn read(&mut self, size: u64) -> Vec<T> {
        let mut buf = (*self.inner).borrow_mut();
        buf.buf.get_mut(&self.id).unwrap().read(size)
    }
    pub fn read_all(&mut self) -> Vec<T> {
        let size = self.size();
        self.read(size)
    }
    pub fn size(&mut self) -> u64 {
        let mut buf = (*self.inner).borrow_mut();
        let bc = buf.get_buffer_cache(self.id).unwrap();
        bc.size
    }

    pub fn id(&self) -> u64 {
        self.id
    }
}
impl<T> MsgQueueWriter<T>
where
    T: Default + Clone,
{
    pub fn write(&self, data: Vec<T>) {
        for (_index, buf) in (*self.inner).borrow_mut().buf.iter_mut() {
            buf.write(data.to_vec());
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum BufferCacheMode {
    Fixed,
    Dynamic,
}

/// BufferCache<T> is implemented with a multi-block circular buffer.
struct BufferCache<T> {
    cache: Vec<Vec<T>>,
    mode: BufferCacheMode,
    buf_length: u64,
    page_size: u64,
    w_index: u64,
    r_index: u64,
    size: u64,
    w_page_index: u64,
    r_page_index: u64,
}

//using capacity()-1 == size() as the sign of buf is full.
impl<T> BufferCache<T>
where
    T: Default + Clone,
{
    pub fn new() -> BufferCache<T> {
        let page_size = 4096;
        let buf_length = 2;
        let buf_cache = vec![vec![T::default(); page_size]; buf_length];
        BufferCache {
            cache: buf_cache,
            mode: Fixed,
            buf_length: buf_length as u64, //default: two buffer blocks
            page_size: page_size as u64,   //page size is 4k
            w_index: 0,                    //
            r_index: 0,
            size: 0,
            w_page_index: 0,
            r_page_index: 0,
        }
    }
    //Fixed mode:the coming data will overlap the exist data;
    pub fn write(&mut self, data: Vec<T>) {
        let target_len = data.len() as u64;
        //only Fixed mode need to calculate the
        if target_len > self.capacity() - self.size() {
            if self.mode == Fixed {
                if target_len >= self.capacity() {
                    //only get the capacity size data
                    let start_data_index = target_len - self.capacity() - 1;
                    for i in 0..self.buf_length {
                        for j in 0..self.page_size {
                            self.cache[i as usize][j as usize] =
                                data[(start_data_index + i * self.page_size + j) as usize].clone();
                        }
                    }
                    self.size = self.buf_length * self.page_size - 1;
                    self.r_index = 0;
                    self.r_page_index = 0;
                    self.w_index = self.page_size - 1;
                    self.w_page_index = self.buf_length - 1;
                } else {
                    let mut a_page_index = self.w_page_index;
                    let mut a_index = self.w_index;
                    for i in 0..target_len {
                        self.cache[a_page_index as usize][a_index as usize] =
                            data[i as usize].clone();
                        a_index += 1;
                        if a_index == self.page_size {
                            a_index = 0;
                            a_page_index = (a_page_index + 1) & self.buf_length;
                        }
                    }
                    self.w_page_index = a_page_index;
                    self.w_index = a_index;
                    if a_index + 1 == self.page_size {
                        self.r_index = 0;
                        self.r_page_index = (self.r_page_index + 1) % self.buf_length;
                    } else {
                        self.r_index = a_index + 1;
                        self.r_page_index = a_page_index;
                    }
                    self.size = self.capacity();
                }
                //some data will be overlapped
            } else if self.mode == Dynamic {
                //expand a new vector for store

                // self.buf_length += 1;
                // self.cache.push(vec![0; self.page_size as usize]);

                //length resize
                //ceil((cur length + new data size) / 4096) * 2
                // self.cache.resize()
                self.size += target_len;

                let target_buf_length = (self.buf_length
                    + math::round::ceil(target_len as f64 / self.page_size as f64, 0) as u64)
                    * 2;
                let old_buf_length = self.buf_length;
                self.buf_length = target_buf_length;
                self.cache.resize(
                    target_buf_length as usize,
                    vec![T::default(); self.page_size as usize],
                );
                if self.w_page_index <= self.r_page_index && self.w_index <= self.r_index {
                    //r < w
                    let mut new_w_index = self.w_index;
                    let mut new_w_page_index = old_buf_length;
                    assert_eq!(self.w_index, 0);
                    assert_eq!(self.w_page_index, 0);
                    let mut old_w_index = self.w_index;
                    let mut old_w_page_index = self.w_page_index;
                    for _i in 0..(self.page_size * self.w_page_index + self.w_index) {
                        self.cache[new_w_page_index as usize][new_w_index as usize] =
                            self.cache[old_w_page_index as usize][old_w_index as usize].clone();
                        new_w_index += 1;
                        if new_w_index == self.page_size {
                            new_w_page_index += 1;
                            new_w_index = 0;
                        }
                        old_w_index += 1;
                        if old_w_index == self.page_size {
                            old_w_page_index += 1;
                            old_w_index = 0;
                        }
                    }

                    self.w_page_index = new_w_page_index;
                    self.w_index = new_w_index;
                }

                //w > r
                //move read -> write
                let mut r_index = self.r_index;
                let mut r_page_index = self.r_page_index;

                let mut n_r_index = self.r_index;
                let mut n_r_page_index = self.r_page_index;
                for _i in 0..self.size() {
                    self.cache[n_r_page_index as usize][n_r_index as usize] =
                        self.cache[r_page_index as usize][r_index as usize].clone();
                    r_index += 1;
                    if r_index == self.page_size {
                        r_page_index += 1;
                        r_index = 0;
                    }
                    n_r_index += 1;
                    if n_r_index == self.page_size {
                        n_r_page_index += 1;
                        n_r_index = 0;
                    }
                }

                let mut w_index = self.w_index;
                for i in 0..target_len {
                    self.cache[self.w_page_index as usize][w_index as usize] =
                        data[i as usize].clone();
                    w_index += 1;
                    if w_index == self.page_size {
                        w_index = 0;
                        self.w_page_index += 1;
                    }
                }
            }
            return;
        }
        let mut index = target_len;
        while index != 0 {
            let mut wrote_size = self.page_size - self.w_index;

            let w_index = self.w_index;
            let mut w_page_index = self.w_page_index;

            if index < wrote_size {
                wrote_size = index;
                self.w_index += index;
            } else {
                self.w_page_index = (self.w_page_index + 1) % self.buf_length;
                self.w_index = 0;
            }
            for i in 0..wrote_size {
                //fix me
                self.cache[w_page_index as usize][(w_index + i) as usize] =
                    data[i as usize].clone();
            }
            index -= wrote_size;
        }
        self.size += target_len;
    }

    // current unconsumed data
    pub fn size(&self) -> u64 {
        return self.size;
    }

    //total buf capacity
    pub fn capacity(&self) -> u64 {
        if self.mode == Fixed {
            self.page_size * self.buf_length - 1
        } else {
            //in Dynamic mode, capacity is no meaningful
            //TODO Does Dynamic uses the same strategy like Fixed
            self.page_size * self.buf_length
        }
    }

    pub fn is_full(&self) -> bool {
        self.capacity() == self.size()
    }

    //only read available data
    pub fn read(&mut self, length: u64) -> Vec<T> {
        let mut lens = length;
        //check whether buf has enough data for reading
        if lens > self.size() {
            lens = self.size();
        }
        if lens == 0 {
            return vec![];
        }
        let mut res = vec![];
        while lens != 0 {
            let read_index_start = self.r_index;
            let mut read_index_end = self.r_index;
            let cur_page_readable_size = self.page_size - self.r_index;
            let page_index = self.r_page_index;
            if self.r_page_index == self.w_page_index {
                //in the same page
                if self.r_index > self.w_index {
                    // cache layout
                    //··· free space， --- used space             index
                    // ------------------------------------------  0
                    // ------------------------------------------  1
                    // ------------w_index······r_index----------  2
                    // ------------------------------------------  3
                    // ------------------------------------------  end of cache
                    if cur_page_readable_size > lens {
                        //current page data is enough
                        read_index_end = read_index_start + lens;
                        self.r_index += lens as u64;
                    } else {
                        read_index_end = read_index_start + cur_page_readable_size;
                        self.r_index = 0;
                        self.read_page_add();
                    }
                } else {
                    // cache layout
                    //··· free space， --- used space             index
                    // ··········································  0
                    // ··········································  1
                    // ···········r_index------w_index··········   2
                    // ··········································  3
                    // ··········································  end of cache
                    read_index_end = read_index_start + lens;
                    self.r_index += lens as u64;
                }
            } else {
                // cache layout
                //··· free space， --- used space             index
                // ··········································  0
                // ···r_index--------------------------------  1
                // -------------------------w_index··········  2
                // ··········································  3
                // ··········································  end of cache

                // cache layout
                //··· free space， --- used space             index
                // ------------------------------------------  0
                // ---w_index································  1
                // ·························r_index----------  2
                // ------------------------------------------  3
                // ------------------------------------------  end of cache

                if cur_page_readable_size > lens {
                    read_index_end = read_index_start + lens;
                    self.r_index += lens;
                } else {
                    read_index_end = read_index_start + cur_page_readable_size;
                    self.r_index = 0;
                    self.read_page_add();
                }
            }

            let rs = read_index_start as usize;
            let re = read_index_end as usize;
            res.append(
                self.cache[page_index as usize][rs..re]
                    .to_vec()
                    .clone()
                    .as_mut(),
            );
            lens -= read_index_end - read_index_start;
        }
        self.size -= length;
        if self.size == 0 {
            //reset index
            self.w_page_index = 0;
            self.w_index = 0;
            self.r_page_index = 0;
            self.r_index = 0;
            //fixme
            //resize in Dynamic mode
        }
        res
    }

    pub fn read_all(&mut self) -> Vec<T> {
        // self.read(self.size())
        vec![]
    }

    fn read_page_add(&mut self) {
        self.r_page_index = (self.r_page_index + 1) % self.buf_length;
    }

    pub fn mode(&self) -> BufferCacheMode {
        self.mode
    }

    pub fn set_fixed_mode(&mut self, buf_length: u64, page_size: u64) {
        self.buf_length = buf_length;
        self.page_size = page_size;
        self.cache = vec![vec![T::default(); page_size as usize]; buf_length as usize];
        self.mode = Fixed;
        self.w_index = 0;
        self.r_index = 0;
        self.size = 0;
        self.w_page_index = 0;
        self.r_page_index = 0;
    }
    pub fn set_dynamic_mode(&mut self, page_size: u64) {
        self.buf_length = 2; //default buf length is 2
        self.page_size = page_size;
        self.cache = vec![vec![T::default(); page_size as usize]; self.buf_length as usize];
        self.mode = Dynamic;
        self.w_index = 0;
        self.r_index = 0;
        self.size = 0;
        self.w_page_index = 0;
        self.r_page_index = 0;
    }

    pub fn readable(&self) -> bool {
        self.size() != 0
    }
}

#[cfg(test)]
mod tests {
    use crate::core::{BufferCache, BufferCacheMode, MsgQueue};
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::{Arc, Mutex};
    use std::thread;

    // #[test]
    fn test_buff_cache() {
        let mut buf = BufferCache::new();
        assert_eq!(buf.mode(), BufferCacheMode::Fixed);
        assert_eq!(buf.size(), 0);
        assert_eq!(buf.capacity(), 4096 * 2 - 1);
        assert_eq!(buf.read(3).len(), 0);
        buf.write(vec![10, 12]);
        assert_eq!(buf.size(), 2);
        assert_eq!(buf.read(2).len(), 2);

        buf.write(vec![10, 12]);
        buf.write(vec![10, 12]);
        buf.write(vec![10, 12]);
        buf.write(vec![10, 12]);
        buf.write(vec![255, 12, 1, 2, 3, 4, 5, 6, 2]);

        buf.write(vec![0; 4096 * 2]);
        assert!(buf.is_full());
        buf.read_all();
        assert_eq!(buf.size(), 0);
        buf.write(vec![0; 4096 * 3]);
        assert!(buf.is_full());
        buf.read(4096);
        assert_eq!(buf.size(), 4095);
    }

    // #[test]
    fn test_overlap() {
        let mut buf = BufferCache::new();
        println!("start");
        buf.write(vec![0; 6000]);
        println!("end");
        assert_eq!(buf.w_index, 6000 - 4096);
        assert_eq!(buf.w_page_index, 1);
        //read 0,0 write 1,4095
        buf.write(vec![0; 4096 * 3]);

        //read 0,2000 write 1,4095
        buf.read(2000);

        assert_eq!(buf.r_index, 2000);
        //read 0,2000 write 0,999
        buf.write(vec![0; 1000]);
        assert_eq!(buf.r_index, 2000);
        assert_eq!(buf.r_page_index, 0);
        assert_eq!(buf.w_index, 999);
        assert_eq!(buf.w_page_index, 0);
        //read 1,
        buf.write(vec![0; 3095]);
        assert_eq!(buf.is_full(), true);
        assert_eq!(buf.r_index, 4095);
        assert_eq!(buf.w_index, 4094);
        assert_eq!(buf.r_page_index, 0);
        assert_eq!(buf.w_page_index, 0);

        buf.read(200);
        assert_eq!(buf.r_index, 199);
        assert_eq!(buf.w_index, 4094);
        assert_eq!(buf.r_page_index, 1);
        assert_eq!(buf.w_page_index, 0);

        buf.write(vec![0; 100]);
        assert_eq!(buf.r_index, 199);
        assert_eq!(buf.w_index, 98);
        assert_eq!(buf.r_page_index, 1);
        assert_eq!(buf.w_page_index, 1);
    }

    #[test]
    fn test_dynamic_mode() {
        let mut buf = BufferCache::new();
        buf.set_dynamic_mode(4096);
        buf.write(vec![0; 4096 * 2]);
        assert_eq!(buf.is_full(), true);
        assert_eq!(buf.size(), 4096 * 2);
        buf.write(vec![0; 1]);
        assert_eq!(buf.size(), 4096 * 2 + 1);
        assert_eq!(buf.capacity(), 4096 * 6);
    }

    #[test]
    fn test_single_thread_msg_queue() {
        let mut msg_queue: MsgQueue<u8> = MsgQueue::new();
        let mut writer1 = msg_queue.add_producer();
        let mut read1 = msg_queue.add_consumer();
        writer1.write(vec![10; 100]);
        println!("{}", msg_queue.get_consumer_count());
        println!("{}", read1.size());
        assert_eq!(read1.size(), 100);
        let mut read2 = msg_queue.add_consumer();
        assert_eq!(read2.size(), 0);
        assert_eq!(msg_queue.get_consumer_count(), 2);
        writer1.write(vec![0; 100]);
        assert_eq!(read1.size(), 200);
        assert_eq!(read2.size(), 100);
        read2.read(50);
        assert_eq!(read1.size(), 200);
        assert_eq!(read2.size(), 50);
    }

    #[test]
    fn test_multi_thread_msg_queue() {
        let mut msg_queue: Arc<Mutex<MsgQueue<u8>>> = Arc::new(Mutex::new(MsgQueue::new()));
        let m1 = msg_queue.clone();
        let m2 = msg_queue.clone();
        let mut c1_id = 0;
        let mut c2_id = 0;
        {
            let mut msg_lock = (*msg_queue).lock().unwrap();
            let mut c1 = msg_lock.add_consumer();
            let mut c2 = msg_lock.add_consumer();
            c1_id = c1.id();
            c2_id = c2.id();
        }
        assert_eq!(msg_queue.lock().unwrap().get_consumer_count(), 2);
        let t1 = thread::spawn(move || {
            let mut msg_lock = (*m1).lock().unwrap();
            println!("get lock1");
            let p = msg_lock.add_producer();
            for i in 0..100 {
                p.write(vec![0; 5]);
            }
        });

        let t2 = thread::spawn(move || {
            let mut msg_lock = (*m2).lock().unwrap();
            println!("get lock1");
            let p = msg_lock.add_producer();
            for i in 0..100 {
                p.write(vec![0; 5]);
            }
        });
        t1.join();
        t2.join();
        {
            let mut msg_lock = (*msg_queue).lock().unwrap();
            assert_eq!(msg_lock.get_consumer_count(), 2);
            let mut c1 = msg_lock.get_consumer(c1_id);
            let mut c2 = msg_lock.get_consumer(c2_id);
            println!("size: {} {}", c1.size(), c2.size());
            assert_eq!(c1.size(), 1000);
            assert_eq!(c2.size(), 1000);
        }
    }

    #[test]
    fn test_string() {
        let mut msg_queue = Rc::new(RefCell::new(MsgQueue::<String>::new()));
        let mut c1 = msg_queue.borrow_mut().add_consumer();
        let mut p1 = msg_queue.borrow_mut().add_producer();
        p1.write(vec!["hello".to_string(), "world".to_string()]);
        assert_eq!(c1.size(), 2);
        let data = c1.read_all();
        assert_eq!(c1.size(), 0);
        assert_eq!(data.len(), 2);
        assert_eq!(data.get(0).unwrap().to_string(), "hello".to_string());
        assert_eq!(data.get(1).unwrap().to_string(), "world".to_string());
        for i in data {
            print!("{:?} ", i);
        }
    }

    #[test]
    fn test_config() {
        let mut msg_queue = Rc::new(RefCell::new(MsgQueue::<String>::new()));
        assert_eq!(msg_queue.borrow_mut().is_running(), false);
        msg_queue.borrow_mut().set_dynamic(4096);
        let mut c1 = msg_queue.borrow_mut().add_consumer();
        let mut p1 = msg_queue.borrow_mut().add_producer();
        assert_eq!(msg_queue.borrow_mut().is_running(), true);
        p1.write(vec!["hello".to_string(), "world".to_string()]);
    }

    #[test]
    #[should_panic]
    fn test_config_and_get_panic() {
        let mut msg_queue = Rc::new(RefCell::new(MsgQueue::<String>::new()));
        assert_eq!(msg_queue.borrow_mut().is_running(), false);
        let mut c1 = msg_queue.borrow_mut().add_consumer();
        let mut p1 = msg_queue.borrow_mut().add_producer();
        assert_eq!(msg_queue.borrow_mut().is_running(), true);
        msg_queue.borrow_mut().set_dynamic(4096);
    }
}
