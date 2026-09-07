use hook_macro::cvt_hook_start;

fn hook() {
    ();
}

fn hook2() {
    ();
}

#[cvt_hook_start(hook(), hook2())]
fn t1() {
    // hook inserted here
    println!("t1");
}

fn main() {}