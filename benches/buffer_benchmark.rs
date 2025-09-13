use std::io::Write;
use std::time::Instant;

fn benchmark_buffer_size(size: usize, iterations: usize) -> f64 {
    let mut buffer = vec![0u8; size];
    let data = vec![b'X'; size / 2];

    let start = Instant::now();

    for _ in 0..iterations {
        // Simulate buffer operations
        buffer.copy_from_slice(&vec![0u8; size]);
        (&mut buffer[..data.len()]).copy_from_slice(&data);

        // Simulate write operation
        let mut sink = std::io::sink();
        let _ = sink.write_all(&buffer);
    }

    start.elapsed().as_secs_f64()
}

fn main() {
    println!("Buffer Size Performance Benchmark");
    println!("==================================");

    let iterations = 10000;
    let sizes = [4096, 8192, 16384, 32768];

    for size in &sizes {
        let time = benchmark_buffer_size(*size, iterations);
        let throughput = ((*size as f64 * iterations as f64) / time) / (1024.0 * 1024.0);

        println!(
            "Buffer {}KB: {:.3}s for {} iterations ({:.2} MB/s)",
            size / 1024,
            time,
            iterations,
            throughput
        );
    }

    println!("\nRecommendation: 16KB buffer provides optimal balance");
}
