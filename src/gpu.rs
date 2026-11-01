// Keep GPU sampling on the system worker; no shell processes run per sample.
#[cfg(windows)]
pub use windows::Monitor;

#[cfg(target_os = "linux")]
pub use linux::{Monitor, card};

#[cfg(not(any(windows, target_os = "linux")))]
pub struct Monitor;

#[cfg(not(any(windows, target_os = "linux")))]
impl Monitor {
    pub fn new() -> Self {
        Self
    }
    pub fn sample(&mut self) -> Option<f32> {
        None
    }
}

/// The primary graphics card as (model, memory capacity).
#[cfg(not(any(windows, target_os = "linux")))]
pub fn card() -> Option<(String, String)> {
    None
}

#[cfg(any(windows, test))]
fn busiest_engine(samples: impl Iterator<Item = (String, f64)>) -> Option<f32> {
    let mut engines = std::collections::HashMap::<String, f64>::new();
    for (instance, usage) in samples {
        if !usage.is_finite() || usage < 0.0 {
            continue;
        }
        // Instances identify a process, adapter LUID, physical GPU, and engine.
        // Sum processes sharing an engine, keeping different adapters separate.
        let Some((_, adapter)) = instance.split_once("_luid_") else {
            continue;
        };
        let Some((engine, _)) = adapter.split_once("_engtype_") else {
            continue;
        };
        *engines.entry(engine.to_owned()).or_default() += usage;
    }
    engines
        .into_values()
        .reduce(f64::max)
        .map(|v| v.clamp(0.0, 100.0) as f32)
}

#[cfg(windows)]
mod windows {
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::System::Performance::*;

    pub struct Monitor {
        query: PDH_HQUERY,
        counter: PDH_HCOUNTER,
    }

    impl Monitor {
        pub fn new() -> Self {
            let mut monitor = Self {
                query: null_mut(),
                counter: null_mut(),
            };
            let path: Vec<u16> = "\\GPU Engine(*)\\Utilization Percentage\0"
                .encode_utf16()
                .collect();
            // SAFETY: handles are initialized by PDH and path is null terminated.
            unsafe {
                if PdhOpenQueryW(null(), 0, &mut monitor.query) != 0 {
                    return monitor;
                }
                if PdhAddEnglishCounterW(monitor.query, path.as_ptr(), 0, &mut monitor.counter) != 0
                {
                    monitor.counter = null_mut();
                    return monitor;
                }
                // Rate counters need a baseline before the worker's first interval.
                PdhCollectQueryData(monitor.query);
            }
            monitor
        }

        pub fn sample(&mut self) -> Option<f32> {
            if self.counter.is_null() {
                return None;
            }
            // SAFETY: this worker exclusively owns these live query/counter handles.
            unsafe {
                if PdhCollectQueryData(self.query) != 0 {
                    return None;
                }
                // The instance list may grow between sizing and reading. Retry from
                // a fresh sizing call rather than trusting a failed read's size.
                for _ in 0..3 {
                    let mut size = 0;
                    let mut count = 0;
                    if PdhGetFormattedCounterArrayW(
                        self.counter,
                        PDH_FMT_DOUBLE,
                        &mut size,
                        &mut count,
                        null_mut(),
                    ) != PDH_MORE_DATA
                    {
                        return None;
                    }
                    if size == 0 || size > 32 * 1024 * 1024 {
                        return None;
                    }
                    // Struct storage provides the alignment required by PDH and
                    // space for both the item array and its trailing UTF-16 names.
                    let slots = (size as usize)
                        .div_ceil(std::mem::size_of::<PDH_FMT_COUNTERVALUE_ITEM_W>());
                    let mut buffer: Vec<_> =
                        std::iter::repeat_with(PDH_FMT_COUNTERVALUE_ITEM_W::default)
                            .take(slots)
                            .collect();
                    let status = PdhGetFormattedCounterArrayW(
                        self.counter,
                        PDH_FMT_DOUBLE,
                        &mut size,
                        &mut count,
                        buffer.as_mut_ptr(),
                    );
                    if status == PDH_MORE_DATA {
                        continue;
                    }
                    if status != 0 || count as usize > buffer.len() {
                        return None;
                    }
                    let samples = buffer[..count as usize].iter().filter_map(|item| {
                        if !matches!(
                            item.FmtValue.CStatus,
                            PDH_CSTATUS_VALID_DATA | PDH_CSTATUS_NEW_DATA
                        ) || item.szName.is_null()
                        {
                            return None;
                        }
                        // PDH returns null-terminated names within the owned buffer.
                        // Bound decoding to that allocation as an additional guard.
                        let start = item.szName as usize;
                        let end =
                            buffer.as_ptr() as usize + std::mem::size_of_val(buffer.as_slice());
                        if start < buffer.as_ptr() as usize
                            || start >= end
                            || !start.is_multiple_of(2)
                        {
                            return None;
                        }
                        let limit = (end - start) / 2;
                        let length = (0..limit).find(|i| *item.szName.add(*i) == 0)?;
                        let wide = std::slice::from_raw_parts(item.szName, length);
                        Some((
                            String::from_utf16_lossy(wide),
                            item.FmtValue.Anonymous.doubleValue,
                        ))
                    });
                    return super::busiest_engine(samples);
                }
            }
            None
        }
    }

    impl Drop for Monitor {
        fn drop(&mut self) {
            if !self.query.is_null() {
                // SAFETY: this uniquely owned query closes its counters too.
                unsafe {
                    PdhCloseQuery(self.query);
                }
            }
        }
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::ffi::{CString, c_int, c_uint, c_void};
    use std::path::{Path, PathBuf};

    const NVIDIA: &str = "0x10de";

    enum Source {
        /// amdgpu exposes a ready-made busy percentage.
        Busy(PathBuf),
        /// NVIDIA proprietary driver, sampled through NVML by PCI bus id.
        Nvidia { bus_id: CString, power: PathBuf },
    }

    pub struct Monitor {
        sources: Vec<Source>,
        nvml: Option<Nvml>,
    }

    impl Monitor {
        pub fn new() -> Self {
            let sources: Vec<_> = devices().iter().filter_map(|d| source(d)).collect();
            let nvml = sources
                .iter()
                .any(|s| matches!(s, Source::Nvidia { .. }))
                .then(Nvml::load)
                .flatten();
            Self { sources, nvml }
        }

        pub fn sample(&mut self) -> Option<f32> {
            self.sources
                .iter()
                .filter_map(|source| match source {
                    Source::Busy(path) => read(path)?.parse::<f32>().ok(),
                    Source::Nvidia { bus_id, power } => {
                        // Querying a runtime-suspended laptop GPU would wake it up.
                        if read(power).as_deref() == Some("suspended") {
                            return Some(0.0);
                        }
                        self.nvml.as_ref()?.utilization(bus_id)
                    }
                })
                .map(|v| v.clamp(0.0, 100.0))
                .reduce(f32::max)
        }
    }

    /// The graphics card with the most dedicated memory, as (model, capacity),
    /// matching the Windows inventory's choice and format.
    pub fn card() -> Option<(String, String)> {
        let mut nvml = None;
        devices()
            .iter()
            .filter_map(|device| {
                let vendor = read(&device.join("vendor"))?;
                let bus = bus_id(device)?;
                let memory = if vendor == NVIDIA {
                    // A one-off read at startup; NVML is closed again right after.
                    nvml.get_or_insert_with(Nvml::load)
                        .as_ref()
                        .and_then(|n| n.memory(&CString::new(bus.as_str()).ok()?))
                } else {
                    read(&device.join("mem_info_vram_total")).and_then(|v| v.parse().ok())
                };
                let model = nvidia_model(&bus)
                    .or_else(|| read(&device.join("product_name")).filter(|n| !n.is_empty()))
                    .or_else(|| pci_name(&vendor, &read(&device.join("device"))?))?;
                Some((model, memory))
            })
            .max_by_key(|(_, memory)| memory.unwrap_or(0))
            .map(|(model, memory)| {
                let capacity = match memory {
                    Some(bytes) if bytes > 0 => {
                        format!("{:.0} GiB", bytes as f64 / (1u64 << 30) as f64)
                    }
                    _ => "Capacity unavailable".into(),
                };
                (model, capacity)
            })
    }

    /// PCI device directories of the DRM cards, skipping connectors such as card1-eDP-1.
    fn devices() -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir("/sys/class/drm") else {
            return Vec::new();
        };
        let mut cards: Vec<_> = entries
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| {
                n.strip_prefix("card")
                    .is_some_and(|i| i.parse::<u32>().is_ok())
            })
            .collect();
        cards.sort();
        cards
            .into_iter()
            .map(|card| Path::new("/sys/class/drm").join(card).join("device"))
            .collect()
    }

    fn bus_id(device: &Path) -> Option<String> {
        let path = std::fs::canonicalize(device).ok()?;
        Some(path.file_name()?.to_str()?.to_owned())
    }

    fn source(device: &Path) -> Option<Source> {
        let busy = device.join("gpu_busy_percent");
        if busy.exists() {
            return Some(Source::Busy(busy));
        }
        if read(&device.join("vendor"))? != NVIDIA {
            return None;
        }
        Some(Source::Nvidia {
            bus_id: CString::new(bus_id(device)?).ok()?,
            power: device.join("power/runtime_status"),
        })
    }

    fn nvidia_model(bus: &str) -> Option<String> {
        let info = std::fs::read_to_string(format!("/proc/driver/nvidia/gpus/{bus}/information"));
        info.ok()?.lines().find_map(|line| {
            let model = line.strip_prefix("Model:")?.trim();
            (!model.is_empty()).then(|| model.to_owned())
        })
    }

    /// Looks the device up in the system PCI ID database, preferring the
    /// marketing name in brackets, e.g. "GA107BM [GeForce RTX 3050 Ti Mobile]".
    fn pci_name(vendor: &str, device: &str) -> Option<String> {
        let ids = ["/usr/share/hwdata/pci.ids", "/usr/share/misc/pci.ids"]
            .iter()
            .find_map(|path| std::fs::read_to_string(path).ok())?;
        pci_lookup(&ids, vendor.strip_prefix("0x")?, device.strip_prefix("0x")?)
    }

    pub(super) fn pci_lookup(ids: &str, vendor: &str, device: &str) -> Option<String> {
        let mut lines = ids.lines();
        let vendor_name = lines.find_map(|l| l.strip_prefix(vendor)?.strip_prefix("  "))?;
        let model = lines
            .take_while(|l| l.starts_with('\t') || l.starts_with('#') || l.is_empty())
            .find_map(|l| {
                l.strip_prefix('\t')?
                    .strip_prefix(device)?
                    .strip_prefix("  ")
            })?;
        let model = model
            .split_once('[')
            .and_then(|(_, rest)| rest.split_once(']'))
            .map_or(model, |(inner, _)| inner);
        let vendor = match vendor {
            "10de" => "NVIDIA",
            "1002" => "AMD",
            "8086" => "Intel",
            _ => vendor_name,
        };
        Some(format!("{vendor} {model}"))
    }

    fn read(path: &Path) -> Option<String> {
        Some(std::fs::read_to_string(path).ok()?.trim().to_owned())
    }

    #[repr(C)]
    #[derive(Default)]
    struct Utilization {
        gpu: c_uint,
        memory: c_uint,
    }

    #[repr(C)]
    #[derive(Default)]
    struct Memory {
        total: u64,
        free: u64,
        used: u64,
    }

    struct Nvml {
        init: unsafe extern "C" fn() -> c_int,
        shutdown: unsafe extern "C" fn() -> c_int,
        by_bus_id: unsafe extern "C" fn(*const std::ffi::c_char, *mut *mut c_void) -> c_int,
        utilization: unsafe extern "C" fn(*mut c_void, *mut Utilization) -> c_int,
        memory: unsafe extern "C" fn(*mut c_void, *mut Memory) -> c_int,
        _library: libloading::Library,
    }

    impl Nvml {
        fn load() -> Option<Self> {
            // SAFETY: libnvidia-ml has no load-time side effects, and each symbol is
            // bound to the signature documented in nvml.h.
            unsafe {
                let library = libloading::Library::new("libnvidia-ml.so.1").ok()?;
                Some(Self {
                    init: *library.get(b"nvmlInit_v2\0").ok()?,
                    shutdown: *library.get(b"nvmlShutdown\0").ok()?,
                    by_bus_id: *library.get(b"nvmlDeviceGetHandleByPciBusId_v2\0").ok()?,
                    utilization: *library.get(b"nvmlDeviceGetUtilizationRates\0").ok()?,
                    memory: *library.get(b"nvmlDeviceGetMemoryInfo\0").ok()?,
                    _library: library,
                })
            }
        }

        fn utilization(&self, bus_id: &CString) -> Option<f32> {
            let mut rates = Utilization::default();
            // SAFETY: `rates` matches nvmlUtilization_t and outlives the call.
            self.with_device(bus_id, |device| unsafe {
                (self.utilization)(device, &mut rates)
            })?;
            Some(rates.gpu as f32)
        }

        fn memory(&self, bus_id: &CString) -> Option<u64> {
            let mut memory = Memory::default();
            // SAFETY: `memory` matches nvmlMemory_t and outlives the call.
            self.with_device(bus_id, |device| unsafe {
                (self.memory)(device, &mut memory)
            })?;
            Some(memory.total)
        }

        /// Opens NVML only for the duration of one query: an open NVML session
        /// keeps the GPU out of runtime suspend, which drains laptop batteries.
        fn with_device(
            &self,
            bus_id: &CString,
            query: impl FnOnce(*mut c_void) -> c_int,
        ) -> Option<()> {
            // SAFETY: the functions come from the still-loaded library; the handle
            // is only used between a successful init and the matching shutdown.
            unsafe {
                if (self.init)() != 0 {
                    return None;
                }
                let mut device = std::ptr::null_mut();
                let ok = (self.by_bus_id)(bus_id.as_ptr(), &mut device) == 0 && query(device) == 0;
                (self.shutdown)();
                ok.then_some(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::busiest_engine;

    #[cfg(target_os = "linux")]
    #[test]
    fn pci_lookup_prefers_marketing_name_within_vendor() {
        let ids = "# comment\n10de  NVIDIA Corporation\n\t25df  Other\n\t25e0  GA107BM [GeForce RTX 3050 Ti Mobile]\n\t\t1043 1ccc  Subsystem\n10df  Emulex\n\t25e1  Wrong vendor\n";
        let lookup = super::linux::pci_lookup;
        assert_eq!(
            lookup(ids, "10de", "25e0").as_deref(),
            Some("NVIDIA GeForce RTX 3050 Ti Mobile")
        );
        assert_eq!(lookup(ids, "10de", "25df").as_deref(), Some("NVIDIA Other"));
        assert_eq!(lookup(ids, "10de", "25e1"), None);
        assert_eq!(
            lookup(ids, "10df", "25e1").as_deref(),
            Some("Emulex Wrong vendor")
        );
    }

    #[test]
    fn combines_processes_per_engine_and_keeps_gpus_separate() {
        let samples = [
            ("pid_1_luid_0x0_0x1_phys_0_eng_0_engtype_3D", 30.0),
            ("pid_2_luid_0x0_0x1_phys_0_eng_0_engtype_3D", 40.0),
            ("pid_1_luid_0x0_0x1_phys_0_eng_1_engtype_Copy", 20.0),
            ("pid_1_luid_0x0_0x2_phys_0_eng_0_engtype_3D", 50.0),
        ];
        assert_eq!(
            busiest_engine(samples.into_iter().map(|(k, v)| (k.into(), v))),
            Some(70.0)
        );
    }

    #[test]
    fn distinguishes_idle_from_unavailable_and_rejects_bad_samples() {
        let key = "pid_1_luid_0x0_0x1_phys_0_eng_0_engtype_3D";
        assert_eq!(
            busiest_engine(std::iter::once((key.into(), 0.0))),
            Some(0.0)
        );
        assert_eq!(busiest_engine(std::iter::empty()), None);
        for value in [f64::NAN, f64::INFINITY, -1.0] {
            assert_eq!(busiest_engine(std::iter::once((key.into(), value))), None);
        }
        assert_eq!(
            busiest_engine(std::iter::once((key.into(), 101.0))),
            Some(100.0)
        );
    }
}
