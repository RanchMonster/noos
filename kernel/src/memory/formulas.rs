use super::*;

pub const MAX_ORDER: usize = 18;

pub const fn size_to_order(size: usize) -> usize {
    // A 0-byte allocation or anything less than a single page fits in Order 0
    if size <= PAGE_SIZE {
        return 0;
    }

    // 1. Calculate how many pages are required (rounding up)
    let pages = (size + PAGE_SIZE - 1) / PAGE_SIZE;

    // 2. Find the next power of two order
    // (pages - 1) handles exact power-of-two boundaries correctly
    let order = (usize::BITS - (pages - 1).leading_zeros()) as usize;

    order
}

pub const fn order_to_size(order: usize) -> usize {
    assert!(order <= MAX_ORDER, "order exceeds MAX_ORDER");
    PAGE_SIZE.checked_shl(order as u32).expect("order_to_size overflow")
}

pub const fn order_to_pages(order: usize) -> usize {
    1 << order
}

pub const fn size_to_pages(size: usize) -> usize {
    (size + PAGE_SIZE - 1) / PAGE_SIZE
}

// Mathematically, these should be true
const _: () = assert!(
    size_to_order(PAGE_SIZE) == 0,
    "PAGE_SIZE must be equal to the zeroth order"
);
const _: () = assert!(
    order_to_size(0) == PAGE_SIZE,
    "PAGE_SIZE must be equal to the zeroth order"
);
const _: () = assert!(
    size_to_order(PAGE_SIZE * 4) == 2,
    "4 Pages must be equal to the second order"
);
const _: () = assert!(
    order_to_size(2) == PAGE_SIZE * 4,
    "4 Pages must be equal to the second order"
);