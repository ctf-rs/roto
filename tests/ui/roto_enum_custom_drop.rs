use roto::RotoEnum;

#[derive(RotoEnum)]
enum CustomDrop {
    Value(u32),
}

impl Drop for CustomDrop {
    fn drop(&mut self) {}
}

fn main() {}
