use super::{CancelOrderParams, PlaceOrderParams};
use crate::state::OrderType;
use borsh::BorshDeserialize;
use hypertree::DataIndex;
use pinocchio::error::ProgramError;

/// A validated view of the existing Borsh encoding. Keep input records in the
/// instruction buffer instead of allocating and copying two vectors.
pub(super) struct BatchUpdateView<'a> {
    pub trader_index_hint: Option<DataIndex>,
    cancels: Cancels<'a>,
    orders: Orders<'a>,
    global_order_counts: [usize; 2],
}

fn take<'a>(data: &mut &'a [u8], len: usize) -> Result<&'a [u8], ProgramError> {
    if data.len() < len {
        return Err(ProgramError::BorshIoError);
    }
    let (head, tail) = data.split_at(len);
    *data = tail;
    Ok(head)
}

fn hint(data: &mut &[u8]) -> Result<Option<DataIndex>, ProgramError> {
    match take(data, 1)?[0] {
        0 => Ok(None),
        1 => Ok(Some(u32::from_le_bytes(take(data, 4)?.try_into().unwrap()))),
        _ => Err(ProgramError::BorshIoError),
    }
}

impl<'a> BatchUpdateView<'a> {
    pub fn read(mut data: &'a [u8]) -> Result<Self, ProgramError> {
        let trader_index_hint = hint(&mut data)?;
        let num_cancels = u32::from_le_bytes(take(&mut data, 4)?.try_into().unwrap());
        let cancel_start = data;
        for _ in 0..num_cancels {
            match take(&mut data, 9)?[8] {
                0 => {}
                1 => {
                    take(&mut data, 4)?;
                }
                _ => return Err(ProgramError::BorshIoError),
            }
        }
        let cancel_bytes = &cancel_start[..cancel_start.len() - data.len()];
        let num_orders = u32::from_le_bytes(take(&mut data, 4)?.try_into().unwrap());
        let order_bytes = (num_orders as usize)
            .checked_mul(19)
            .ok_or(ProgramError::BorshIoError)?;
        if data.len() != order_bytes {
            return Err(ProgramError::BorshIoError);
        }
        let mut global_order_counts = [0usize; 2];
        // The exact length check above guarantees complete records. Arrays
        // keep their validated width in the type and need only a slice iterator.
        let (records, _) = data.as_chunks::<19>();
        for record in records {
            // Borsh validates bool, but OrderType is a transparent u8. Keep
            // semantic order-type and price checks in the processing loop.
            if record[13] > 1 {
                return Err(ProgramError::BorshIoError);
            }
            if record[18] == OrderType::Global.as_u8() {
                global_order_counts[record[13] as usize] += 1;
            }
        }
        Ok(Self {
            trader_index_hint,
            global_order_counts,
            cancels: Cancels { data: cancel_bytes },
            orders: Orders {
                records: records.iter(),
            },
        })
    }

    pub fn into_parts(self) -> (Option<DataIndex>, Cancels<'a>, Orders<'a>, [usize; 2]) {
        (
            self.trader_index_hint,
            self.cancels,
            self.orders,
            self.global_order_counts,
        )
    }
}

pub(super) struct Cancels<'a> {
    data: &'a [u8],
}
impl Iterator for Cancels<'_> {
    type Item = CancelOrderParams;
    fn next(&mut self) -> Option<Self::Item> {
        let (prefix, tail) = self.data.split_first_chunk::<9>()?;
        let order_sequence_number = u64::from_le_bytes(prefix[..8].try_into().unwrap());
        let order_index_hint = if prefix[8] == 1 {
            let (hint, tail) = tail.split_at(4);
            self.data = tail;
            Some(u32::from_le_bytes(hint.try_into().unwrap()))
        } else {
            self.data = tail;
            None
        };
        Some(CancelOrderParams {
            order_sequence_number,
            order_index_hint,
        })
    }
}

#[derive(Clone)]
pub(super) struct Orders<'a> {
    records: std::slice::Iter<'a, [u8; 19]>,
}
impl Iterator for Orders<'_> {
    type Item = PlaceOrderParams;
    fn next(&mut self) -> Option<Self::Item> {
        let record = self.records.next()?;
        Some(PlaceOrderParams {
            base_atoms: u64::from_le_bytes(record[..8].try_into().unwrap()),
            price_mantissa: u32::from_le_bytes(record[8..12].try_into().unwrap()),
            price_exponent: record[12] as i8,
            is_bid: record[13] == 1,
            last_valid_slot: u32::from_le_bytes(record[14..18].try_into().unwrap()),
            order_type: BorshDeserialize::deserialize(&mut &record[18..19]).unwrap(),
        })
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.records.size_hint()
    }
}
impl ExactSizeIterator for Orders<'_> {
    fn len(&self) -> usize {
        self.records.len()
    }
}

#[cfg(test)]
mod tests {
    use super::{super::BatchUpdateParams, *};
    use crate::state::OrderType;
    use borsh::BorshSerialize;

    fn check(data: &[u8]) {
        let reference = BatchUpdateParams::try_from_slice(data);
        let parsed = BatchUpdateView::read(data);
        assert_eq!(reference.is_ok(), parsed.is_ok(), "data: {data:?}");
        if let (Ok(reference), Ok(parsed)) = (reference, parsed) {
            let (seat, cancels, orders, counts) = parsed.into_parts();
            let expected_counts = reference
                .orders
                .iter()
                .fold([0usize; 2], |mut counts, order| {
                    if order.order_type() == OrderType::Global {
                        counts[usize::from(order.is_bid())] += 1;
                    }
                    counts
                });
            assert_eq!(counts, expected_counts);
            let actual = BatchUpdateParams::new(seat, cancels.collect(), orders.collect());
            assert_eq!(
                actual.try_to_vec().unwrap(),
                reference.try_to_vec().unwrap()
            );
        }
    }

    fn fixture() -> Vec<u8> {
        BatchUpdateParams::new(
            Some(u32::MAX),
            vec![
                CancelOrderParams::new_with_hint(u64::MAX, Some(80)),
                CancelOrderParams::new(0),
            ],
            vec![
                PlaceOrderParams::new(u64::MAX, u32::MAX, -18, true, OrderType::Limit, 0),
                PlaceOrderParams::new(0, 1, 8, false, OrderType::Global, u32::MAX),
                PlaceOrderParams::new(1, 2, -1, true, OrderType::ReverseTight, 17),
            ],
        )
        .try_to_vec()
        .unwrap()
    }

    #[test]
    fn wire_view_matches_borsh_valid_and_truncated() {
        let data = fixture();
        for end in 0..=data.len() {
            check(&data[..end]);
        }
        for seat in [None, Some(0), Some(u32::MAX)] {
            check(
                &BatchUpdateParams::new(seat, vec![], vec![])
                    .try_to_vec()
                    .unwrap(),
            );
        }
        let mut trailing = data.clone();
        trailing.push(0);
        check(&trailing);
        // None seat and None cancel hints have different wire lengths.
        check(
            &BatchUpdateParams::new(None, vec![CancelOrderParams::new(1)], vec![])
                .try_to_vec()
                .unwrap(),
        );
    }

    #[test]
    fn wire_view_matches_borsh_generated_batches() {
        let mut state = 0x243f6a8885a308d3u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for case in 0..1024 {
            let seat = if case % 2 == 0 {
                Some(next() as u32)
            } else {
                None
            };
            let cancels = (0..case % 9)
                .map(|index| {
                    let sequence = next();
                    let hint = if index % 2 == 0 {
                        Some(next() as u32)
                    } else {
                        None
                    };
                    CancelOrderParams::new_with_hint(sequence, hint)
                })
                .collect();
            let orders = (0..case % 13)
                .map(|_| {
                    let amount = next();
                    let mantissa = next() as u32;
                    let exponent = next() as i8;
                    let bid = next() & 1 != 0;
                    let slot = next() as u32;
                    // Include every raw order-type byte: wire decoding accepts
                    // them, and processing performs the semantic validation.
                    let raw_type = [next() as u8];
                    let order_type = OrderType::deserialize(&mut &raw_type[..]).unwrap();
                    PlaceOrderParams::new(amount, mantissa, exponent, bid, order_type, slot)
                })
                .collect();
            check(
                &BatchUpdateParams::new(seat, cancels, orders)
                    .try_to_vec()
                    .unwrap(),
            );
        }
    }

    #[test]
    fn wire_view_matches_borsh_mutations() {
        let original = fixture();
        for index in 0..original.len() {
            for value in 0..=u8::MAX {
                let mut data = original.clone();
                data[index] = value;
                check(&data);
            }
        }
        let mut state = 0x9e3779b97f4a7c15u64;
        for len in 0..256 {
            for _ in 0..16 {
                let data: Vec<u8> = (0..len)
                    .map(|_| {
                        state ^= state << 13;
                        state ^= state >> 7;
                        state ^= state << 17;
                        state as u8
                    })
                    .collect();
                check(&data);
            }
        }
    }
}
