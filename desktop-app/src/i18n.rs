use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Language {
    ZhCn,
    #[default]
    En,
}

impl Language {
    pub const fn strings(self) -> &'static Strings {
        match self {
            Self::ZhCn => &ZH_CN,
            Self::En => &EN,
        }
    }

    pub fn format_source(self, source: &str) -> String {
        let strings = self.strings();
        if let Some(source) = source.strip_prefix("configured:") {
            return format!("{}{}", strings.configured_source, self.source_name(source));
        }
        self.source_name(source).to_string()
    }

    fn source_name(self, source: &str) -> &str {
        let strings = self.strings();
        match source {
            "auto" => strings.source_auto,
            "pipewire" => strings.source_pipewire,
            "screencapturekit" => strings.source_screencapturekit,
            "tone" => strings.source_tone,
            "idle" => strings.source_idle,
            _ => source,
        }
    }
}

pub struct Strings {
    pub subtitle: &'static str,
    pub running: &'static str,
    pub stopped: &'static str,
    pub starting: &'static str,
    pub stopping: &'static str,
    pub degraded: &'static str,
    pub disconnecting: &'static str,
    pub start_service: &'static str,
    pub stop_service: &'static str,
    pub refresh: &'static str,
    pub copy_address: &'static str,
    pub pair_device: &'static str,
    pub settings: &'static str,
    pub discovery_port: &'static str,
    pub control_address: &'static str,
    pub control_port: &'static str,
    pub audio_format: &'static str,
    pub phone_target: &'static str,
    pub waiting_for_phone: &'static str,
    pub audio_source: &'static str,
    pub rtp_packets: &'static str,
    pub bytes_sent: &'static str,
    pub logs: &'static str,
    pub clear_logs: &'static str,
    pub copy_diagnostics: &'static str,
    pub connected_device: &'static str,
    pub no_connected_device: &'static str,
    pub disconnect_device: &'static str,
    pub scan_to_pair: &'static str,
    pub pairing_address: &'static str,
    pub close: &'static str,
    pub save: &'static str,
    pub cancel: &'static str,
    pub start_on_launch: &'static str,
    pub minimize_to_tray: &'static str,
    pub audio_source_setting: &'static str,
    pub packet_duration: &'static str,
    pub control_port_range: &'static str,
    pub discovery_port_range: &'static str,
    pub port_range_separator: &'static str,
    pub tray_show: &'static str,
    pub tray_quit: &'static str,
    pub service_path: &'static str,
    pub service_ready: &'static str,
    pub service_stopped: &'static str,
    pub service_exited: &'static str,
    pub service_status_error: &'static str,
    pub service_status_restored: &'static str,
    pub start_failed: &'static str,
    pub address_copied: &'static str,
    pub diagnostics_copied: &'static str,
    pub service_started: &'static str,
    pub device_disconnected: &'static str,
    pub settings_saved: &'static str,
    pub restart_required: &'static str,
    pub operation_failed: &'static str,
    pub service_output: &'static str,
    pub service_not_found: &'static str,
    pub service_stdout_unavailable: &'static str,
    pub service_ready_timeout: &'static str,
    pub unable_to_start: &'static str,
    pub configured_source: &'static str,
    pub source_auto: &'static str,
    pub source_pipewire: &'static str,
    pub source_screencapturekit: &'static str,
    pub source_tone: &'static str,
    pub source_idle: &'static str,
}

static ZH_CN: Strings = Strings {
    subtitle: "局域网手机扬声器",
    running: "运行中",
    stopped: "未运行",
    starting: "启动中",
    stopping: "停止中",
    degraded: "状态异常",
    disconnecting: "断开中",
    start_service: "启动服务",
    stop_service: "停止服务",
    refresh: "刷新",
    copy_address: "复制地址",
    pair_device: "配对设备",
    settings: "设置",
    discovery_port: "发现端口",
    control_address: "控制地址",
    control_port: "控制端口",
    audio_format: "音频格式",
    phone_target: "手机目标",
    waiting_for_phone: "等待手机",
    audio_source: "当前音源",
    rtp_packets: "RTP 包",
    bytes_sent: "发送量",
    logs: "日志",
    clear_logs: "清空",
    copy_diagnostics: "复制诊断信息",
    connected_device: "已连接设备",
    no_connected_device: "暂无已连接设备",
    disconnect_device: "断开设备",
    scan_to_pair: "使用手机扫描二维码配对",
    pairing_address: "配对地址",
    close: "关闭",
    save: "保存",
    cancel: "取消",
    start_on_launch: "打开桌面端时启动服务",
    minimize_to_tray: "关闭窗口时最小化到托盘",
    audio_source_setting: "音频来源",
    packet_duration: "音频包时长",
    control_port_range: "控制端口范围",
    discovery_port_range: "发现端口范围",
    port_range_separator: "至",
    tray_show: "显示窗口",
    tray_quit: "退出",
    service_path: "后台服务",
    service_ready: "服务就绪",
    service_stopped: "服务已停止",
    service_exited: "服务已退出",
    service_status_error: "服务状态错误",
    service_status_restored: "服务状态已恢复",
    start_failed: "启动失败",
    address_copied: "控制地址已复制",
    diagnostics_copied: "诊断信息已复制",
    service_started: "服务已启动",
    device_disconnected: "设备已断开",
    settings_saved: "设置已保存",
    restart_required: "服务设置将在下次启动时生效",
    operation_failed: "操作失败",
    service_output: "服务输出",
    service_not_found: "找不到 lanpulse-service；请先构建后台服务或设置 LANPULSE_SERVICE_PATH",
    service_stdout_unavailable: "后台服务 stdout 未打开",
    service_ready_timeout: "等待后台服务启动超时",
    unable_to_start: "无法启动",
    configured_source: "已配置：",
    source_auto: "自动",
    source_pipewire: "PipeWire",
    source_screencapturekit: "ScreenCaptureKit",
    source_tone: "测试音",
    source_idle: "空闲",
};

static EN: Strings = Strings {
    subtitle: "LAN phone speaker",
    running: "Running",
    stopped: "Stopped",
    starting: "Starting",
    stopping: "Stopping",
    degraded: "Degraded",
    disconnecting: "Disconnecting",
    start_service: "Start Service",
    stop_service: "Stop Service",
    refresh: "Refresh",
    copy_address: "Copy Address",
    pair_device: "Pair Device",
    settings: "Settings",
    discovery_port: "Discovery Port",
    control_address: "Control Address",
    control_port: "Control Port",
    audio_format: "Audio Format",
    phone_target: "Phone Target",
    waiting_for_phone: "Waiting for phone",
    audio_source: "Audio Source",
    rtp_packets: "RTP Packets",
    bytes_sent: "Bytes Sent",
    logs: "Logs",
    clear_logs: "Clear",
    copy_diagnostics: "Copy Diagnostics",
    connected_device: "Connected Device",
    no_connected_device: "No device connected",
    disconnect_device: "Disconnect",
    scan_to_pair: "Scan with your phone to pair",
    pairing_address: "Pairing Address",
    close: "Close",
    save: "Save",
    cancel: "Cancel",
    start_on_launch: "Start service when the app opens",
    minimize_to_tray: "Minimize to tray when closing the window",
    audio_source_setting: "Audio Source",
    packet_duration: "Packet Duration",
    control_port_range: "Control Port Range",
    discovery_port_range: "Discovery Port Range",
    port_range_separator: "to",
    tray_show: "Show Window",
    tray_quit: "Quit",
    service_path: "Service",
    service_ready: "Ready",
    service_stopped: "Service stopped",
    service_exited: "Service exited",
    service_status_error: "Service status error",
    service_status_restored: "Service status restored",
    start_failed: "Start failed",
    address_copied: "Control address copied",
    diagnostics_copied: "Diagnostics copied",
    service_started: "Service started",
    device_disconnected: "Device disconnected",
    settings_saved: "Settings saved",
    restart_required: "Service settings will apply on the next start",
    operation_failed: "Operation failed",
    service_output: "Service output",
    service_not_found: "lanpulse-service was not found; build it first or set LANPULSE_SERVICE_PATH",
    service_stdout_unavailable: "Service stdout is unavailable",
    service_ready_timeout: "Timed out waiting for the service",
    unable_to_start: "Unable to start",
    configured_source: "Configured: ",
    source_auto: "Auto",
    source_pipewire: "PipeWire",
    source_screencapturekit: "ScreenCaptureKit",
    source_tone: "Test tone",
    source_idle: "Idle",
};

#[cfg(test)]
mod tests {
    use super::Language;

    #[test]
    fn defaults_to_english() {
        assert_eq!(Language::default(), Language::En);
    }

    #[test]
    fn translates_known_audio_sources() {
        assert_eq!(
            Language::ZhCn.format_source("configured:auto"),
            "已配置：自动"
        );
        assert_eq!(Language::En.format_source("tone"), "Test tone");
    }

    #[test]
    fn unknown_audio_sources_are_displayed_verbatim() {
        assert_eq!(
            Language::En.format_source("configured:alsa"),
            "Configured: alsa"
        );
        assert_eq!(Language::ZhCn.format_source("custom"), "custom");
    }
}
