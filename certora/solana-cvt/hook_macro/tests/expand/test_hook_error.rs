use hook_macro::cvt_hook_start;

fn hook() {
    ();
}

// adding this hook to a struct should result
// in a compile error
#[cvt_hook_start(hook())]
struct S1 {
    a: i32
}

fn main() {}