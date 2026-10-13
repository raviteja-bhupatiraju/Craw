use std::time::Instant;
use std::hint::black_box;

fn fib(n: i64) -> i64 {
    if n <= 1 {
        return n;
    }
    fib(n - 1) + fib(n - 2)
}

fn primes(n: usize) -> i64 {
    let mut sieve = vec![true; n + 1];
    let mut p = 2;
    while p * p <= n {
        if sieve[p] {
            let mut i = p * p;
            while i <= n {
                sieve[i] = false;
                i += p;
            }
        }
        p += 1;
    }
    
    let mut count = 0;
    for i in 2..=n {
        if sieve[i] {
            count += 1;
        }
    }
    count
}

fn matmul(size: usize) -> i64 {
    let mut a = vec![vec![0; size]; size];
    let mut b = vec![vec![0; size]; size];
    let mut c = vec![vec![0; size]; size];

    for i in 0..size {
        for j in 0..size {
            a[i][j] = (i + j) as i64;
            b[i][j] = (i as i64) - (j as i64);
        }
    }

    for i in 0..size {
        for j in 0..size {
            for k in 0..size {
                c[i][j] += a[i][k] * b[k][j];
            }
        }
    }
    c[0][0]
}

fn main() {
    // Warmup
    black_box(fib(black_box(10)));
    black_box(primes(black_box(100)));
    black_box(matmul(black_box(10)));
    
    println!("--- Rust Benchmark ---");
    
    let start1 = Instant::now();
    let res1 = black_box(fib(black_box(30)));
    let end1 = start1.elapsed().as_millis();
    println!("Fib(30) = {} | Time: {} ms", res1, end1);

    let start2 = Instant::now();
    let res2 = black_box(primes(black_box(1000000)));
    let end2 = start2.elapsed().as_millis();
    println!("Primes(1000000) = {} | Time: {} ms", res2, end2);

    let start3 = Instant::now();
    let res3 = black_box(matmul(black_box(200)));
    let end3 = start3.elapsed().as_millis();
    println!("Matmul(200x200) = {} | Time: {} ms", res3, end3);
}
