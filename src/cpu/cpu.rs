use std::time::Instant;

use crate::cpu::executor::{CpuExecutor, CpuExecutorImpl};
use crate::cpu::interfaces::{BusInterface, LlmInterface, MemoryInterface, SkillInterface};
use crate::cpu::state::AgentState;

use crate::hy_evo::integration::HyEvoIntegration;
use crate::hy_evo::scoring::ExecutionMetrics;

pub struct Cpu<'a> {
    pub state: AgentState,

    pub memory: &'a mut dyn MemoryInterface,
    pub skills: &'a dyn SkillInterface,
    pub llm: &'a dyn LlmInterface,
    pub bus: &'a dyn BusInterface,

    pub hyevo: HyEvoIntegration<()>, // replace () with your logger type later
}
impl<L> Cpu<L>
where
    L: ReflectionLlm + Send + Sync + 'static,
{
    pub fn execute_instruction(&mut self, instr: Instruction) {
        println!("Executing instruction: {:?}", instr);  // Added logging
        match instr {
            Instruction::RunSkill { name, args } => {
                match self.skills.call(&name, args, self.memory, self.bus) {
                    Ok(_) => {},
                    Err(e) => eprintln!("Error running skill: {}", e),  // Added error checking
                }
            }

            Instruction::ExecuteHooks { phase } => {
                let _ = self.hooks.execute(&phase, self.memory, self.bus);
            }

            Instruction::PlanNextSteps => {
                // TODO: planning logic
            }

            Instruction::ReflectOnLastStep => {
                // TODO: reflection logic
            }

            Instruction::WaitForEvent => {
                // idle
            }
        }
    }
}

impl<'a> Cpu<'a> {
    pub fn new(
        memory: &'a mut dyn MemoryInterface,
        skills: &'a dyn SkillInterface,
        llm: &'a dyn LlmInterface,
        bus: &'a dyn BusInterface,
        hyevo: HyEvoIntegration<()>,
    ) -> Self {
        Self {
            state: AgentState::new(),
            memory,
            skills,
            llm,
            bus,
            hyevo,
        }
    }

    /// Main heartbeat handler — called every X ms by your scheduler
    pub fn handle_heartbeat(&mut self) {
        println!("Handling heartbeat: Tick {}", self.state.tick_count);  // Added logging
        self.state.bump_tick();
        self.state.last_heartbeat = Instant::now();
        self.state.uptime = self.state.start_time.elapsed().unwrap_or_default();

        // Write heartbeat.md
        if let Err(e) = self.write_heartbeat_file() {
            eprintln!("Failed to write heartbeat.md: {}", e);  // Enhanced error logging
        }

        // Trigger HyEvo every 10 ticks
        if self.state.tick_count % 10 == 0 {
            if let Err(e) = self.run_hyevo_cycle() {
                eprintln!("Error in HyEvo cycle: {}", e);  // Added error checking
            }
        }
    }

    /// Run a HyEvo evolution cycle
    pub fn run_hyevo_cycle(&mut self) -> anyhow::Result<()> {
        let executor = CpuExecutorImpl {
            memory: self.memory,
            skills: self.skills,
            llm: self.llm,
            bus: self.bus,
        };

        self.hyevo.run_and_evolve(&executor)
    }

    /// Writes heartbeat.md
    fn write_heartbeat_file(&self) -> std::io::Result<()> {
        use std::fs;

        let md = format!(
            "# Heartbeat\n\n\             Tick: {}\n\             Uptime: {:?}\n\             Mode: {:?}\n\             Errors: {}\n",
            self.state.tick_count, self.state.uptime, self.state.mode, self.state.error_count,
        );

        fs::write("heartbeat.md", md)
    }
}

impl<L> Cpu<L>
where
    L: ReflectionLlm + Send + Sync + 'static,
{
    pub fn handle_bus_message(&mut self, msg: Message) {
        // 1. Log it
        println!("CPU received message: {:?}", msg);

        // 2. Convert bus message → CPU event
        let event = CpuEvent::from_message(&msg);

        // 3. Run scheduler to get instructions
        let instructions = Scheduler::schedule(&self.state, Some(event));

        // 4. Execute instructions
        for instr in instructions {
            self.execute_instruction(instr);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cpu_new_initializes_state() {
        // Create a mock or dummy implementation for testing
        struct DummyInterface;
        impl MemoryInterface for DummyInterface {}
        impl SkillInterface for DummyInterface {}
        impl LlmInterface for DummyInterface {}
        impl BusInterface for DummyInterface {}

        let mut dummy_memory = DummyInterface;
        let dummy_skills = DummyInterface;
        let dummy_llm = DummyInterface;
        let dummy_bus = DummyInterface;
        let hyevo = HyEvoIntegration::<()>::new();  // Assuming a new method or default

        let cpu = Cpu::new(&mut dummy_memory, &dummy_skills, &dummy_llm, &dummy_bus, hyevo);
        assert_eq!(cpu.state.tick_count, 0);  // Assuming AgentState::new() sets tick_count to 0
    }

    #[test]
    fn test_handle_heartbeat_increments_tick() {
        struct DummyInterface;
        impl MemoryInterface for DummyInterface {}
        impl SkillInterface for DummyInterface {}
        impl LlmInterface for DummyInterface {}
        impl BusInterface for DummyInterface {}

        let mut dummy_memory = DummyInterface;
        let dummy_skills = DummyInterface;
        let dummy_llm = DummyInterface;
        let dummy_bus = DummyInterface;
        let hyevo = HyEvoIntegration::<()>::new();  // Assuming a new method or default

        let mut cpu = Cpu::new(&mut dummy_memory, &dummy_skills, &dummy_llm, &dummy_bus, hyevo);
        cpu.handle_heartbeat();
        assert_eq!(cpu.state.tick_count, 1);  // Check if tick_count was incremented
    }
}