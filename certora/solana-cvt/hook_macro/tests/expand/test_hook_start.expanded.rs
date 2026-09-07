use hook_macro::cvt_hook_start;
fn hook() {
    ();
}
fn t1() {
    hook();
    {
        ::std::io::_print(format_args!("t1\n"));
    };
}
