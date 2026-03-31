use std::fs::File;
use std::io::{Read, Write};

fn main() {
    let mut input = File::open("frames_8bit.bin").unwrap();
    let mut output = File::create("frames.bin").unwrap();

    let mut buffer = [0u8; 160 * 120];
    while let Ok(n) = input.read_exact(&mut buffer) {
        let mut packed = Vec::with_capacity(160 * 120 / 8);
        for chunk in buffer.chunks(8) {
            let mut byte = 0u8;
            for (i, &pixel) in chunk.iter().enumerate() {
                if pixel > 128 {
                    byte |= 1 << (7 - i);
                }
            }
            packed.push(byte);
        }
        output.write_all(&packed).unwrap();
    }
}
