use hook_macro::cvt_hook_end;

fn hook() {
    ();
}

#[cvt_hook_end(hook())]
fn t1() {
    assert_eq!(1, 1);
    // hook inserted here
    assert_eq!(2, 2);
}

#[cvt_hook_end(hook())]
fn t2() {
    // hook inserted here
    assert_eq!(1, 1);
}

#[cvt_hook_end(hook())]
fn tmp() -> Result<()> {
    // hook inserted here
    Ok(())
}

fn t3() {
    assert_eq!(tmp(), Ok(()));
}