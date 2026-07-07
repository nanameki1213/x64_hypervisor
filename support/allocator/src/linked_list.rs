use core::marker::PhantomData;

#[derive(Clone, Copy)]
pub struct LinkedList {
    head: *mut usize,
}

unsafe impl Send for LinkedList {}

#[allow(dead_code)]
impl LinkedList {
    pub const fn new() -> Self {
        LinkedList {
            head: core::ptr::null_mut(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.head.is_null()
    }

    pub fn push(&mut self, value: *mut usize) {
        unsafe {
            *value = self.head as usize;
        }
        self.head = value;
    }

    pub fn pop(&mut self) -> Option<*mut usize> {
        if self.is_empty() {
            None
        } else {
            let value = self.head;
            self.head = unsafe { *(self.head) as *mut usize };
            Some(value)
        }
    }

    pub fn iter(&self) -> LinkedListIter<'_> {
        LinkedListIter {
            curr: self.head,
            list: PhantomData,
        }
    }

    pub fn iter_mut(&self) -> LinkedListIterMut<'_> {
        LinkedListIterMut {
            list: PhantomData,
            prev: core::ptr::null_mut(),
            curr: self.head,
        }
    }
}

pub struct LinkedListIter<'a> {
    curr: *mut usize,
    list: PhantomData<&'a LinkedList>,
}

impl<'a> Iterator for LinkedListIter<'a> {
    type Item = *mut usize;

    fn next(&mut self) -> Option<Self::Item> {
        if self.curr.is_null() {
            None
        } else {
            let item = self.curr;
            let next = unsafe { *item as *mut usize };
            self.curr = next;
            Some(item)
        }
    }
}

pub struct ListNode {
    prev: *mut usize,
    curr: *mut usize,
}

impl ListNode {
    pub fn pop(self) -> *mut usize {
        unsafe {
            *(self.prev) = *(self.curr);
        }
        self.curr
    }

    pub fn value(&self) -> *mut usize {
        self.curr
    }
}

pub struct LinkedListIterMut<'a> {
    list: PhantomData<&'a mut LinkedList>,
    prev: *mut usize,
    curr: *mut usize,
}

impl<'a> Iterator for LinkedListIterMut<'a> {
    type Item = ListNode;

    fn next(&mut self) -> Option<Self::Item> {
        if self.curr.is_null() {
            None
        } else {
            let res = ListNode {
                prev: self.prev,
                curr: self.curr,
            };
            self.prev = self.curr;
            self.curr = unsafe { *self.curr as *mut usize };
            Some(res)
        }
    }
}
