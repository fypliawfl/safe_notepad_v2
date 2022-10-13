fn main() {
    for (gist_id, _) in gist::collect().unwrap() {
        gist::remove(&gist_id).unwrap();
    }
}
