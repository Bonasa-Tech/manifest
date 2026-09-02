use std::marker::PhantomData;
use std::ops::Index;
use nondet::{
    Nondet, nondet, nondet_pointer
};
use cvt::CVT_assume;
use std::io::Read;
use anchor_lang::prelude::borsh::maybestd::io::Write;
use anchor_lang::{AnchorSerialize, AnchorDeserialize};
use borsh::{BorshDeserialize, BorshSerialize};

macro_rules! assert {
    ($cond:expr) => {{ /*cvt::CVT_assert($cond)*/}};
}

pub struct NoDataVec<T> {
    len: usize,
    capacity: usize,
    _marker: PhantomData<T>
}

impl<T: Nondet> NoDataVec<T> {

    fn grow(&mut self, additional: usize) {
        let new_len = self.len + additional;
        let mut new_cap = self.capacity * 2;

        if new_cap < new_len {
            new_cap = new_len;
        }

        assert!(new_cap < usize::MAX);
        self.capacity = new_cap;
    }

    pub fn new() -> Self {
        Self {
            len: 0,
            capacity: 0,
            _marker: PhantomData::default()
        }
    }

    pub fn new_with_len(len: usize) -> Self {
        Self {
            len,
            capacity: len,
            _marker: PhantomData::default()
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn clone(&self) -> Self {
        Self {
            len: self.len, 
            capacity: self.capacity,
            _marker: PhantomData::default()
        }
    }

    pub fn push(&mut self, _value: T) {
        if self.len == self.capacity {
            self.grow(1);
        }
        self.len += 1;
        assert!(self.capacity >= self.len);
    }

    pub fn pop(&mut self) -> Option<T> {
        if self.len == 0 {
            None
        } else {
            self.len -= 1;
            Some(nondet::<T>())
        }
    }

    pub fn insert(&mut self, _index: usize, _value: T) {
        if self.len == self.capacity {
            self.grow(1);
        }
        self.len += 1;
        assert!(self.capacity >= self.len);
    }

    pub fn remove(&mut self, _index: usize) -> T {
        assert!(index < self.len);
        self.len -= 1;
        nondet::<T>()
    }

    pub fn get(&self, _index: usize) -> Option<&T> {
        assert!( index < self.len);
        let ptr: *mut T = nondet_pointer::<T>();
        unsafe {
            Some(&*ptr)
        }
    }

    pub fn binary_search_by_key<'a, B, F>(&'a self, _b: &B, _f: F) -> Result<usize, usize>
    where
        F: FnMut(&'a T) -> B,
        B: Ord,
    {
        let index = nondet::<usize>();
        CVT_assume(index < self.len);
        let tmp: i8 = nondet();
        if tmp > 0 {
            Ok(index)
        } else {
            Err(index)
        }
    }


    pub fn sort_by_key<B, F>(&mut self, _f: F)
    where
        F: FnMut(&T) -> B,
        B: Ord,
    {
        // do nothing
    }

}

////////////////////////
/// Iterator
////////////////////////
pub struct NoDataVecIter<T> {
    start: usize,
    end: usize,
    _marker: PhantomData<T>
}

impl<T: Nondet> Iterator for NoDataVecIter<T> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.start == self.end {
            None
        } else {
            self.start += 1;
            Some(nondet::<T>())
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.end - self.start, Some(self.end - self.start))
    }
}

impl<T: Nondet> IntoIterator for NoDataVec<T> {
    type Item = T;
    type IntoIter = NoDataVecIter<T>;

    fn into_iter(self) -> NoDataVecIter<T> {
        NoDataVecIter {
            start: 0,
            end: self.len,
            _marker: PhantomData::default()
        }
    }
}

/////////////////////////
/// Other traits:
/// - Clone
/// - Index
/// - Nondet
/// - BorshSerialize
/// - BorshDeserialize
/////////////////////////

impl <T: Nondet> Clone for NoDataVec<T> {
    fn clone(&self) -> Self {
        self.clone()
    }
}

impl<T: Nondet> Index<usize> for NoDataVec<T> {
    type Output = T;

    fn index(&self, index: usize) -> &T {
        self.get(index).unwrap()
    }
}

impl<T: Nondet> Nondet for NoDataVec<T> {
    fn nondet() -> Self {
        let len: usize = nondet::<usize>();
        CVT_assume(len < usize::MAX);

        NoDataVec::new_with_len(len)
    }
}

impl<T: AnchorSerialize> BorshSerialize for NoDataVec<T> {
    fn serialize<W: Write>(&self, writer: &mut W) -> borsh::maybestd::io::Result<()> {
        let len = usize::try_from(self.len).map_err(|_| std::io::ErrorKind::InvalidInput)?;
        writer.write_all(&len.to_le_bytes())?;

        let cap = usize::try_from(self.capacity).map_err(|_| std::io::ErrorKind::InvalidInput)?;
        writer.write_all(&cap.to_le_bytes())?;

        // nothing to serialize for the elements

        Ok(())
    }
}

impl<T:Nondet + AnchorDeserialize> BorshDeserialize for NoDataVec<T> {
    fn deserialize(buf: &mut &[u8]) -> borsh::maybestd::io::Result<Self> {
        // Deserialize len and capacity
        let len = usize::deserialize(buf)?;
        let capacity = usize::deserialize(buf)?;

        // nothing to deserialize for the elements

        let mut vec = NoDataVec::<T>::new_with_len(len);
        vec.capacity = capacity;

        Ok(vec)
    }

    fn deserialize_reader<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        // Deserialize len and capacity
        let len = usize::deserialize_reader(reader)?;
        let capacity = usize::deserialize_reader(reader)?;

        // nothing to deserialize for the elements

        let mut vec = NoDataVec::<T>::new_with_len(len);
        vec.capacity = capacity;

        Ok(vec)
    }
}


////////////////////////
/// Macros
////////////////////////
#[macro_export]
macro_rules! cvt_no_data_vec {
    ($elem:expr; $n:expr) => {{
        let mut vec = NoDataVec::new();
        for _ in 0..$n {
            vec.push($elem);
        }
        vec
    }};
    ($($x:expr),*) => {{
        let mut vec = NoDataVec::new();
        $(
            vec.push($x);
        )*
        vec
    }};
}