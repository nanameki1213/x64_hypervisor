#![no_std]

mod linked_list;

use core::alloc::Layout;
use core::cmp::{max, min};
use core::ptr::NonNull;

fn align_up(address: usize, align: usize) -> usize {
    (address + align - 1) & !(align - 1)
}

fn prev_power_of_two(num: usize) -> usize {
    1 << (usize::BITS as usize - num.leading_zeros() as usize - 1)
}

pub struct Heap<const ORDER: usize> {
    free_list: [linked_list::LinkedList; ORDER],
}

impl<const ORDER: usize> Default for Heap<ORDER> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const ORDER: usize> Heap<ORDER> {
    pub const fn new() -> Self {
        Heap {
            free_list: [linked_list::LinkedList::new(); ORDER],
        }
    }

    pub fn add_to_heap(&mut self, mut address: usize, mut size: usize) {
        // 最低でもusizeでアライン
        address = align_up(address, size_of::<usize>());
        size &= !size_of::<usize>() - 1;
        let end = address + size;

        let mut current_address = address;

        while current_address + size_of::<usize>() <= end {
            let lowbit = current_address & (!current_address + 1);
            let mut current_size = min(lowbit, prev_power_of_two(end - current_address));

            let mut order = current_size.trailing_zeros() as usize;
            if order > ORDER - 1 {
                order = ORDER - 1;
                current_size = 1 << order;
            }

            self.free_list[order].push(current_address as *mut usize);
            current_address += current_size;
        }
    }

    pub fn init(&mut self, address: usize, size: usize) {
        self.add_to_heap(address, size);
    }

    // TODO: error handling
    pub fn allocate(&mut self, layout: Layout) -> Result<NonNull<u8>, ()> {
        // アロケート単位は最低でもusize
        let size = max(
            layout.size().next_power_of_two(),
            max(layout.align(), size_of::<usize>()),
        );
        let class = size.trailing_zeros() as usize;
        for i in class..self.free_list.len() {
            if !self.free_list[i].is_empty() {
                // メモリブロックを上位から下位に向かって分割し、補充
                for j in (class + 1..i + 1).rev() {
                    if let Some(block) = self.free_list[j].pop() {
                        self.free_list[j - 1].push((block as usize + (1 << (j - 1))) as *mut usize);
                        self.free_list[j - 1].push(block);
                    } else {
                        return Err(());
                    }
                }
                let result =
                    NonNull::new(self.free_list[class].pop().expect("Out of memory") as *mut u8);
                if let Some(result) = result {
                    return Ok(result);
                } else {
                    return Err(());
                }
            }
        }
        Err(())
    }

    pub fn callocate(&mut self, layout: Layout) -> Result<NonNull<u8>, ()> {
        let size = max(
            layout.size().next_power_of_two(),
            max(layout.align(), size_of::<usize>()),
        );
        let memory = self.allocate(layout)?;
        unsafe {
            core::ptr::write_bytes(memory.as_ptr(), 0, size);
        }
        Ok(memory)
    }

    pub fn deallocate(&mut self, ptr: NonNull<u8>, layout: Layout) {
        let size = max(
            layout.size().next_power_of_two(),
            max(layout.align(), size_of::<usize>()),
        );
        let class = size.trailing_zeros() as usize;

        self.free_list[class].push(ptr.as_ptr() as *mut usize);

        let mut current_ptr = ptr.as_ptr() as usize;
        let mut current_class = class;

        while current_class < self.free_list.len() - 1 {
            // アドレスの1ビットだけ反転させて、バディとなるブロックを探す
            let buddy = current_ptr ^ (1 << current_class);
            let mut flag = false;
            for block in self.free_list[current_class].iter_mut() {
                if block.value() as usize == buddy {
                    block.pop();
                    flag = true;
                    break;
                }
            }

            if flag {
                self.free_list[current_class].pop();
                current_ptr = min(current_ptr, buddy);
                current_class += 1;
                self.free_list[current_class].push(current_ptr as *mut usize);
            } else {
                break;
            }
        }
    }
}
