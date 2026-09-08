//! Frozen numeric ABI semantics; C values are compiled by `c-abi-check`.

#[path = "support/frozen_numbers.rs"]
mod numbers;

#[test]
fn every_frozen_number_is_the_one_this_library_defines() {
    for (name, defined, frozen) in numbers::FROZEN {
        assert_eq!(defined, frozen, "ABI numeric contract changed for {name}");
    }
}
