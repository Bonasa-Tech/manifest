use hook_macro::cvt_hook_start;

fn hook() {
    ();
}

#[cvt_hook_start(hook())]
fn t1() {
    // hook inserted here
    println!("t1");
}