// Keep GPU sampling on the system worker; no shell processes run per sample.
#[cfg(windows)]
pub use windows::Monitor;

#[cfg(not(windows))]
pub struct Monitor;

#[cfg(not(windows))]
impl Monitor {
    pub fn new() -> Self {
        Self
    }
    pub fn sample(&mut self) -> Option<f32> {
        None
    }
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

#[cfg(test)]
mod tests {
    use super::busiest_engine;

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
