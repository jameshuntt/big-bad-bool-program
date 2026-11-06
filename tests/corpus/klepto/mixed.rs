async fn foo(x: i32) { bar(foo) }
fn bar(y: u8) { foo(y) }
fn idle() {}
