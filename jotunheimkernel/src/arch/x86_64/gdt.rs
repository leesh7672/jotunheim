#![allow(unused)]

use core::f32::consts;

use spin::Once;
use x86_64::{
    VirtAddr,
    instructions::{
        segmentation::{CS, DS, ES, SS, Segment},
        tables::load_tss,
    },
    structures::gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector},
    structures::tss::TaskStateSegment,
};

#[derive(Copy, Clone)]
pub struct Selectors {
    pub code: SegmentSelector,
    pub data: SegmentSelector,
    pub tss: SegmentSelector, // lower TSS slot (e.g., 0x28)
}

// Singletons
static GDT: Once<GlobalDescriptorTable> = Once::new();
static SELECTORS: Once<Selectors> = Once::new();
static TSS: Once<TaskStateSegment> = Once::new();

const STACK_SIZE: usize = 4 * 1024;

const RSP0_STACK_LEN: usize = STACK_SIZE;
const DF_STACK_LEN: usize = STACK_SIZE;
const PF_STACK_LEN: usize = STACK_SIZE;
const TIMER_STACK_LEN: usize = STACK_SIZE;
const GP_STACK_LEN: usize = STACK_SIZE;
const UD_STACK_LEN: usize = STACK_SIZE;
const BP_STACK_LEN: usize = STACK_SIZE;
const DB_STACK_LEN: usize = STACK_SIZE;

// Early bring-up stacks (replace with per-CPU allocator + guard pages later)
#[unsafe(link_section = ".bss")]
static mut RSP0_STACK: [u8; RSP0_STACK_LEN] = [0; RSP0_STACK_LEN];
#[unsafe(link_section = ".bss")]
static mut DF_STACK: [u8; DF_STACK_LEN] = [0; DF_STACK_LEN];
#[unsafe(link_section = ".bss")]
static mut PF_STACK: [u8; PF_STACK_LEN] = [0; PF_STACK_LEN];
#[unsafe(link_section = ".bss")]
static mut TIMER_STACK: [u8; TIMER_STACK_LEN] = [0; TIMER_STACK_LEN];
#[unsafe(link_section = ".bss")]
static mut GP_STACK: [u8; GP_STACK_LEN] = [0; GP_STACK_LEN];
#[unsafe(link_section = ".bss")]
static mut UD_STACK: [u8; UD_STACK_LEN] = [0; UD_STACK_LEN];
#[unsafe(link_section = ".bss")]
static mut BP_STACK: [u8; BP_STACK_LEN] = [0; BP_STACK_LEN];
#[unsafe(link_section = ".bss")]
static mut DB_STACK: [u8; DB_STACK_LEN] = [0; DB_STACK_LEN];

const IST_DF: u16 = 1;
const IST_PF: u16 = 2;
const IST_TIMER: u16 = 3;
const IST_GP: u16 = 4;
const IST_UD: u16 = 5;
const IST_BP: u16 = 6;
const IST_DB: u16 = 7;

fn top_raw(base: *mut u8, len: usize) -> VirtAddr {
    // Return top-of-stack (16-byte aligned), without forming &/&mut to static mut
    let end = unsafe { base.add(len) } as *const u8;
    VirtAddr::from_ptr(end).align_down(16u64)
}

/// Build + load GDT/TSS once; return selectors. Safe to call multiple times.
pub fn init() {
    if let Some(s) = SELECTORS.get() {
        return;
    }

    // Materialize TSS with real stacks (once)
    let tss_ref = TSS.call_once(|| {
        let mut t = TaskStateSegment::new();

        // Compute tops from raw pointers to avoid &mut refs to static mut (Rust 2024)
        let rsp0_base = core::ptr::addr_of_mut!(RSP0_STACK) as *mut u8;
        let df_base = core::ptr::addr_of_mut!(DF_STACK) as *mut u8;
        let pf_base = core::ptr::addr_of_mut!(PF_STACK) as *mut u8;
        let timer_base = core::ptr::addr_of_mut!(TIMER_STACK) as *mut u8;
        let gp_base = core::ptr::addr_of_mut!(GP_STACK) as *mut u8;
        let ud_base = core::ptr::addr_of_mut!(UD_STACK) as *mut u8;
        let bp_base = core::ptr::addr_of_mut!(BP_STACK) as *mut u8;
        let db_base = core::ptr::addr_of_mut!(DB_STACK) as *mut u8;

        t.privilege_stack_table[0] = top_raw(rsp0_base, RSP0_STACK_LEN); // rsp0
        t.interrupt_stack_table[(IST_DF - 1) as usize] = top_raw(df_base, DF_STACK_LEN); // #DF
        t.interrupt_stack_table[(IST_PF - 1) as usize] = top_raw(pf_base, PF_STACK_LEN); // #PF (optional but useful)
        t.interrupt_stack_table[(IST_TIMER - 1) as usize] = top_raw(timer_base, TIMER_STACK_LEN);
        t.interrupt_stack_table[(IST_GP - 1) as usize] = top_raw(gp_base, GP_STACK_LEN);
        t.interrupt_stack_table[(IST_UD - 1) as usize] = top_raw(ud_base, UD_STACK_LEN);
        t.interrupt_stack_table[(IST_BP - 1) as usize] = top_raw(bp_base, BP_STACK_LEN);
        t.interrupt_stack_table[(IST_DB - 1) as usize] = top_raw(db_base, DB_STACK_LEN);

        t
    });

    // Build temporary GDT and append entries
    let mut tmp = GlobalDescriptorTable::new();
    let code = tmp.append(Descriptor::kernel_code_segment());
    let data = tmp.append(Descriptor::kernel_data_segment());
    let tss = tmp.append(Descriptor::tss_segment(tss_ref));

    // Move into 'static storage and load from that &'static
    let gdt_ref: &'static GlobalDescriptorTable = GDT.call_once(|| tmp);
    unsafe {
        gdt_ref.load();
        CS::set_reg(code);
        DS::set_reg(data);
        ES::set_reg(data);
        SS::set_reg(data);
        load_tss(tss);
    }

    let sels = Selectors { code, data, tss };
    let _ = SELECTORS.call_once(|| sels);
}

// ---- Accessors ----

pub fn selectors() -> Selectors {
    *SELECTORS.get().expect("gdt::init() not called")
}

pub fn code_selector() -> SegmentSelector {
    selectors().code
}

pub fn data_selector() -> SegmentSelector {
    selectors().data
}

pub fn tss_selector() -> SegmentSelector {
    selectors().tss
}