// error-pattern: the evaluated program panicked

#[derive(Debug)]
struct A;

fn main() {
    assert_eq!(&A as *const A as *const (), &() as *const _);
}
