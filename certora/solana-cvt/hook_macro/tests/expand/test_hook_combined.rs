use hook_macro::{cvt_hook_start, cvt_hook_end};

fn hook_start() {
    ();
}

fn hook_end() {
    ();
}


#[cvt_hook_start(hook_start())]
#[cvt_hook_end(hook_end())]
fn tmp() -> Result<()> {
    // hook start inserted here
    // hook end inserted here
    Ok(())
}

fn t3() {
    assert_eq!(tmp(), Ok(()));
}

#[cvt_hook_start(hook_start())]
#[cvt_hook_end(hook_end())]
fn t4() {
    // hook start inserted here
    assert_eq!(1, 1);
    // hook end inserted here
}

#[cvt_hook_start(hook_start())]
fn abs(x : i32) -> i32 {
    // hook start inserted here
    if x >= 0 { 
        println!("x is positive");
        x 
    } else { 
        println!("x is negative");
        -x 
    }
}

#[cvt_hook_end(hook_end())]
fn abs2(x : i32) -> i32 {
    // hook end inserted here
    if x >= 0 { 
        println!("x is positive");
        x 
    } else { 
        println!("x is negative");
        -x 
    }
}