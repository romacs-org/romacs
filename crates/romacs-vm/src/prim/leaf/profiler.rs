use super::*;
impl Leaf<'_> {
    pub(crate) fn profiler_cpu_start(&mut self, interval: i64) -> std::result::Result<(), Fault> {
        self.vm.profiler_cpu_start(interval).map_err(Fault::Error)
    }
    pub(crate) fn profiler_cpu_stop(&mut self) -> bool { self.vm.profiler_cpu_stop() }
    pub(crate) fn profiler_cpu_running(&self) -> bool { self.vm.profiler_cpu_running() }
    pub(crate) fn profiler_cpu_log(&mut self) -> super::super::Out {
        self.vm.profiler_cpu_log().map_err(Fault::Error)
    }
    pub(crate) fn profiler_memory_start(&mut self) -> std::result::Result<(), Fault> {
        self.vm.profiler_memory_start().map_err(Fault::Error)
    }
    pub(crate) fn profiler_memory_stop(&mut self) -> bool { self.vm.profiler_memory_stop() }
    pub(crate) fn profiler_memory_running(&self) -> bool { self.vm.profiler_memory_running() }
    pub(crate) fn profiler_memory_log(&mut self) -> super::super::Out {
        self.vm.profiler_memory_log().map_err(Fault::Error)
    }
    pub(crate) fn function_equal(&self, a: Value, b: Value) -> bool { self.vm.function_equal(a, b) }
}
