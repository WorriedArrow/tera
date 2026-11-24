// every line here must be reported by clippy with the core-crate clippy.toml.

pub fn time() {
    let a = std::time::Instant::now();
    let _ = a.elapsed();
    let _ = std::time::SystemTime::now();
}

pub fn threads() {
    let h = std::thread::spawn(move || 1);
    let _ = h.join();
    std::thread::sleep(std::time::Duration::from_millis(1));
    std::thread::yield_now();
    let _ = std::thread::available_parallelism();
    let _ = std::thread::current();
}

pub fn env_and_process() {
    let _ = std::env::var("X");
    let _ = std::env::args();
    let _ = std::env::current_dir();
    let _ = std::process::id();
}

pub fn fs_and_net() {
    let _ = std::fs::read("x");
    let _ = std::fs::File::open("x");
    let _ = std::fs::rename("a", "b");
    let _ = std::net::TcpStream::connect("127.0.0.1:1");
}

pub fn maps() {
    let mut m = std::collections::HashMap::new();
    m.insert(1u8, 2u8);
    let s: std::collections::HashSet<u8> = std::collections::HashSet::default();
    let _ = s.len();
    let _ = std::hash::RandomState::new();
}

pub fn hidden_default_hasher() {
    use std::collections::HashMap;
    let m: HashMap<u8, u8> = HashMap::default();
    let _ = m.len();
}

pub fn addresses() {
    let a = std::sync::Arc::new(1u8);
    let _ = std::sync::Arc::as_ptr(&a);
}

pub fn globals() {
    static G: std::sync::OnceLock<u8> = std::sync::OnceLock::new();
    let _ = G.get();
    std::thread_local! {
        static T: u8 = const { 0 };
    }
    println!("hi");
}

pub fn blocking() {
    let (tx, rx) = std::sync::mpsc::channel::<u8>();
    drop(tx);
    let _ = rx.recv();
}
