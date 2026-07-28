fn reserve_before_push(vector: &mut Vec<u8>, value: u8) {
    vector.reserve(1);
    vector.push(value);
}
