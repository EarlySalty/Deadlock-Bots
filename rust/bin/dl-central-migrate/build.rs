fn main() {
    println!("cargo:rerun-if-changed=../../crates/dl-central-db/migrations");
}
