import time

def fib(n):
    if n <= 1:
        return n
    return fib(n - 1) + fib(n - 2)

def primes(n):
    sieve = [True] * (n + 1)
    p = 2
    while p * p <= n:
        if sieve[p]:
            i = p * p
            while i <= n:
                sieve[i] = False
                i += p
        p += 1
    count = 0
    for i in range(2, n + 1):
        if sieve[i]:
            count += 1
    return count

def matmul(size):
    A = [[i + j for j in range(size)] for i in range(size)]
    B = [[i - j for j in range(size)] for i in range(size)]
    C = [[0 for _ in range(size)] for _ in range(size)]

    for i in range(size):
        for j in range(size):
            for k in range(size):
                C[i][j] += A[i][k] * B[k][j]
    return C[0][0]

def main():
    # Warmup
    fib(10)
    primes(100)
    matmul(10)
    
    print("--- Python Benchmark ---")
    
    start1 = time.time() * 1000
    res1 = fib(30)
    end1 = time.time() * 1000
    print("Fib(30) =", res1, f"| Time: {int(end1 - start1)} ms")

    start2 = time.time() * 1000
    res2 = primes(1000000)
    end2 = time.time() * 1000
    print("Primes(1000000) =", res2, f"| Time: {int(end2 - start2)} ms")

    start3 = time.time() * 1000
    res3 = matmul(200)
    end3 = time.time() * 1000
    print("Matmul(200x200) =", res3, f"| Time: {int(end3 - start3)} ms")

if __name__ == "__main__":
    main()
