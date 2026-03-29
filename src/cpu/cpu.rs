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
        match instr {
            Instruction::RunSkill { name, args } => {
                let _ = self.skills.call(&name, args, self.memory, self.bus);
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
        self.state.bump_tick();
        self.state.last_heartbeat = Instant::now();
        self.state.uptime = self.state.start_time.elapsed().unwrap_or_default();

        // Write heartbeat.md
        if let Err(e) = self.write_heartbeat_file() {
            eprintln!("Failed to write heartbeat.md: {}", e);
        }

        // Trigger HyEvo every 10 ticks
        if self.state.tick_count % 10 == 0 {
            let _ = self.run_hyevo_cycle();
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
            "# Heartbeat\n\n\
             Tick: {}\n\
             Uptime: {:?}\n\
             Mode: {:?}\n\
             Errors: {}\n",
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
pub fn execute_instruction(&mut self, instr: Instruction) {
    match instr {
        Instruction::RunSkill { name, args } => {
            let _ = self.skills.call(&name, args, self.memory, self.bus);
        }

        Instruction::ExecuteHooks { phase } => {
            let _ = self.hooks.execute(&phase, self.memory, self.bus);
        }

        Instruction::PlanNextSteps => {
            // planning logic
        }

        Instruction::ReflectOnLastStep => {
            // reflection logic
        }

        Instruction::WaitForEvent => {
            // do nothing
        }
    }
}
impl<L> Cpu<L>
where
    L: ReflectionLlm + Send + Sync + 'static,
{
    pub fn handle_bus_message(&mut self, msg: Message) {
        // Log
        println!("CPU received bus message: {:?}", msg);

        // Convert bus message → CPU event
        let event = CpuEvent::from_message(&msg);

        // Ask scheduler what to do
        let instructions = Scheduler::schedule(&self.state, Some(event));

        // Execute each instruction
        for instr in instructions {
            self.execute_instruction(instr);
        }
    }
}
