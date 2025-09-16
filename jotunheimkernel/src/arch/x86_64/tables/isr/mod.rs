pub mod timer;
pub mod debug;
pub mod fault;
pub mod misc;

pub fn init(){
    timer::init();
    debug::init();
    fault::init();
    misc::init();
}