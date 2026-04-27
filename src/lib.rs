use applevisor::{
    gic::GicConfig,
    memory::{MemPerms, Memory},
    vcpu::{ExitReason, Reg, Vcpu},
    vm::{GicEnabled, VirtualMachine, VirtualMachineConfig, VirtualMachineInstance},
};

pub type Unit = Maybe<()>;
pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Maybe<T> = Result<T, Error>;

pub struct SandboxCore {
    vcpu: Vcpu,
    vmem: Memory,
}

pub struct Sandbox {
    vm: VirtualMachineInstance<GicEnabled>,
}

impl Sandbox {
    pub fn new() -> Maybe<Self> {
        let vm_cfg = VirtualMachineConfig::new();
        let gic_cfg = GicConfig::new();
        let machine = VirtualMachine::with_gic(vm_cfg, gic_cfg)?;
        Ok(Self { vm: machine })
    }
}

impl Sandbox {
    pub fn run(&mut self) -> Unit {
        self.vcpu.set_trap_debug_exceptions(true)?;
        self.vcpu.set_trap_debug_reg_accesses(true)?;
        self.vmem.map(0x4000, MemPerms::RWX)?;
        self.vmem.write_u32(0x4000, 0xd2800840)?; // mov x0, #0x42
        self.vmem.write_u32(0x4004, 0xd4200000)?; // brk #0
        self.vcpu.set_reg(Reg::PC, 0x4000)?;
        self.vcpu.run()?;
        let ei = self.vcpu.get_exit_info();
        dbg!(ei);
        assert_eq!(self.vcpu.get_reg(Reg::X0), Ok(0x42));
        assert_eq!(ei.reason, ExitReason::EXCEPTION);
        assert_eq!(ei.exception.syndrome >> 26, 0b111100);
        Ok(())
    }
}
