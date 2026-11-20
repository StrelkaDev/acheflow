//! The only unsafe boundary: pointers into this WASM instance's own linear memory.
pub use serde_json;
pub mod authorization;
#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "acheflow")]
unsafe extern "C" {
    fn input_len() -> i32;
    fn read_input(ptr: i32, capacity: i32) -> i32;
    fn emit_json(ptr: i32, len: i32);
}
#[cfg(target_arch = "wasm32")]
pub fn input_bytes() -> Vec<u8> {
    let len = unsafe { input_len() };
    assert!((0..=1024 * 1024).contains(&len));
    let mut bytes = vec![0u8; len as usize];
    let copied = unsafe { read_input(bytes.as_mut_ptr() as i32, len) };
    assert_eq!(copied, len);
    bytes
}
#[cfg(target_arch = "wasm32")]
pub fn input() -> serde_json::Value {
    serde_json::from_slice(&input_bytes()).expect("valid host JSON")
}
#[cfg(target_arch = "wasm32")]
pub fn emit(value: &serde_json::Value) {
    let bytes = serde_json::to_vec(value).expect("valid effect JSON");
    unsafe {
        emit_json(bytes.as_ptr() as i32, bytes.len() as i32);
    }
}
#[macro_export]
macro_rules! export {
    ($state:ty) => {
        $crate::export!($state, $crate::serde_json::Value, handle);
    };
    ($state:ty, $input:ty, $method:ident) => {
        #[cfg(target_arch="wasm32")]
        std::thread_local! {static STATE:std::cell::RefCell<$state>=std::cell::RefCell::new(<$state>::default());}
        #[cfg(target_arch="wasm32")]
        #[unsafe(no_mangle)]
        pub extern "C" fn handle() {let input: $input=$crate::serde_json::from_slice(&$crate::input_bytes()).expect("valid host JSON");STATE.with(|state|{for effect in state.borrow_mut().$method(input){$crate::emit(&effect);}});}
    };
}

/// Export a named part with independent guest state, even when both parts share
/// one WASM artifact. Hosts choose an entry point before supplying any secrets.
#[macro_export]
macro_rules! export_part {
    ($entry:ident, $state:ty) => {$crate::export_part!($entry,$state,$crate::serde_json::Value,handle);};
    ($entry:ident, $state:ty, $input:ty, $method:ident) => {
        #[cfg(target_arch="wasm32")]
        #[unsafe(no_mangle)]
        pub extern "C" fn $entry() {
            std::thread_local! {static STATE:std::cell::RefCell<$state>=std::cell::RefCell::new(<$state>::default());}
            let input:$input=$crate::serde_json::from_slice(&$crate::input_bytes()).expect("valid host JSON");
            STATE.with(|state|{for effect in state.borrow_mut().$method(input){$crate::emit(&effect);}});
        }
    };
}
