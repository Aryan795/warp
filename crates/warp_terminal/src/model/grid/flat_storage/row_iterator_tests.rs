use std::rc::Rc;

use super::super::FlatStorage;
use super::RowIterator;

#[test]
fn next_keeps_a_live_rc_so_unwrap_or_clone_cannot_take_ownership() {
    let storage = FlatStorage::from_content_using_rows("hello world\n", 7, Some(2));
    let mut iter = RowIterator::new(&storage, 0);

    let yielded = iter.next().expect("should materialize the first row");
    assert_eq!(Rc::strong_count(&yielded), 2);
    assert!(
        Rc::try_unwrap(yielded).is_err(),
        "RowIterator retains an Rc; unwrap_or_clone would deep-clone Row"
    );
}

#[test]
fn next_owned_moves_each_filled_row_out_of_the_iterator() {
    let storage = FlatStorage::from_content_using_rows("hello world\n", 7, Some(2));
    let mut iter = RowIterator::new(&storage, 0);

    let first = iter.next_owned().expect("should take the first row");
    assert_eq!(first.occ, 7);
    assert_eq!(first[0].c, 'h');
    assert_eq!(first[6].c, 'w');
    assert_eq!(Rc::strong_count(&iter.row), 1);

    let second = iter.next_owned().expect("should take the second row");
    assert_eq!(second.occ, 4);
    assert_eq!(second[0].c, 'o');
    assert_eq!(second[3].c, 'd');

    assert!(iter.next_owned().is_none());
}
