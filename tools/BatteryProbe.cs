// Read-only Windows battery diagnostics. No battery SET_INFORMATION requests.
// API: https://learn.microsoft.com/windows/win32/power/ioctl-battery-query-information
using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.IO;
using System.Runtime.InteropServices;
using System.Web.Script.Serialization;
using Microsoft.Win32.SafeHandles;

internal static class BatteryProbe
{
    // GUID_DEVICE_BATTERY and QUERY IOCTLs from Windows SDK shared/poclass.h.
    static Guid BatteryInterface = new Guid("72631e54-78a4-11d0-bcf7-00aa00b7b32a");
    const uint QueryTag = 0x294040, QueryInformation = 0x294044, QueryStatus = 0x29404c;
    [StructLayout(LayoutKind.Sequential)]
    struct InterfaceData { public uint Size; public Guid ClassGuid; public uint Flags; public UIntPtr Reserved; }
    [DllImport("setupapi.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    static extern IntPtr SetupDiGetClassDevs(ref Guid guid, IntPtr enumerator, IntPtr parent, uint flags);
    [DllImport("setupapi.dll", SetLastError = true)]
    static extern bool SetupDiEnumDeviceInterfaces(IntPtr info, IntPtr device, ref Guid guid, uint index, ref InterfaceData data);
    [DllImport("setupapi.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    static extern bool SetupDiGetDeviceInterfaceDetail(IntPtr info, ref InterfaceData data, IntPtr detail, uint size, out uint needed, IntPtr device);
    [DllImport("setupapi.dll", SetLastError = true)]
    static extern bool SetupDiDestroyDeviceInfoList(IntPtr info);
    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    static extern SafeFileHandle CreateFile(string path, uint access, uint sharing, IntPtr security, uint creation, uint flags, IntPtr template);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool DeviceIoControl(SafeFileHandle handle, uint code, byte[] input, uint inputSize, byte[] output, uint outputSize, out uint returned, IntPtr overlapped);

    static byte[] Query(SafeFileHandle handle, uint code, byte[] input, int length)
    {
        byte[] output = new byte[length]; uint returned;
        if (!DeviceIoControl(handle, code, input, (uint)input.Length, output, (uint)length, out returned, IntPtr.Zero))
            throw new Win32Exception(Marshal.GetLastWin32Error());
        if (returned != length) throw new IOException("Unexpected battery response length: " + returned);
        return output;
    }
    static byte[] Words(params uint[] values)
    {
        byte[] result = new byte[values.Length * 4];
        for (int i = 0; i < values.Length; i++) Buffer.BlockCopy(BitConverter.GetBytes(values[i]), 0, result, i * 4, 4);
        return result;
    }
    static void ReadField(Dictionary<string, object> row, string field, Func<object> read)
    {
        try { row[field] = read(); }
        catch (Exception error) {
            row[field] = null;
            row[field + "_error"] = error.Message;
            Win32Exception native = error as Win32Exception;
            if (native != null) row[field + "_win32_error"] = native.NativeErrorCode;
        }
    }
    static object ReadBattery(string path, uint index)
    {
        var row = new Dictionary<string, object>(); row["index"] = index;
        using (SafeFileHandle handle = CreateFile(path, 0x80000000, 3, IntPtr.Zero, 3, 0x80, IntPtr.Zero)) {
            if (handle.IsInvalid) throw new Win32Exception(Marshal.GetLastWin32Error());
            // Zero timeout: never wait for a missing/replaced battery.
            uint tag = BitConverter.ToUInt32(Query(handle, QueryTag, Words(0), 4), 0);
            if (tag == 0) throw new IOException("Battery is not present");
            ReadField(row, "temperature", delegate {
                uint raw = BitConverter.ToUInt32(Query(handle, QueryInformation, Words(tag, 2, 0), 4), 0);
                double celsius = raw / 10.0 - 273.15;
                if (raw == 0 || raw == UInt32.MaxValue || celsius < -20 || celsius > 100)
                    throw new IOException("Invalid battery temperature payload: " + raw);
                return new { raw_tenths_kelvin = raw, celsius = Math.Round(celsius, 2) };
            });
            ReadField(row, "information", delegate {
                byte[] data = Query(handle, QueryInformation, Words(tag, 0, 0), 36);
                return new {
                    capacity_is_relative = (BitConverter.ToUInt32(data, 0) & 0x40000000) != 0,
                    designed_capacity_raw = BitConverter.ToUInt32(data, 12),
                    full_charge_capacity_raw = BitConverter.ToUInt32(data, 16),
                    cycle_count_raw = BitConverter.ToUInt32(data, 32)
                };
            });
            ReadField(row, "status", delegate {
                byte[] data = Query(handle, QueryStatus, Words(tag, 0, 0, 0, UInt32.MaxValue), 16);
                int rate = BitConverter.ToInt32(data, 12); uint voltage = BitConverter.ToUInt32(data, 8);
                return new {
                    power_state_raw = BitConverter.ToUInt32(data, 0),
                    remaining_capacity_raw = BitConverter.ToUInt32(data, 4),
                    voltage_millivolts = voltage == UInt32.MaxValue ? (uint?)null : voltage,
                    rate_raw = rate == Int32.MinValue ? (int?)null : rate
                };
            });
        }
        return row;
    }
    static int Main(string[] args)
    {
        try {
            if (args.Length != 2 || args[0] != "--output") throw new ArgumentException("Usage: BatteryProbe.exe --output new-report.json");
            using (var file = new FileStream(args[1], FileMode.CreateNew, FileAccess.Write)) {
                var batteries = new List<object>();
                IntPtr info = SetupDiGetClassDevs(ref BatteryInterface, IntPtr.Zero, IntPtr.Zero, 0x12);
                if (info == new IntPtr(-1)) throw new Win32Exception(Marshal.GetLastWin32Error());
                try {
                    for (uint index = 0; index < 8; index++) {
                        var data = new InterfaceData(); data.Size = (uint)Marshal.SizeOf(typeof(InterfaceData));
                        if (!SetupDiEnumDeviceInterfaces(info, IntPtr.Zero, ref BatteryInterface, index, ref data)) {
                            int error = Marshal.GetLastWin32Error();
                            if (error == 259) break;
                            throw new Win32Exception(error);
                        }
                        uint needed;
                        SetupDiGetDeviceInterfaceDetail(info, ref data, IntPtr.Zero, 0, out needed, IntPtr.Zero);
                        if (needed < 8 || needed > 65536) throw new IOException("Invalid battery interface length");
                        IntPtr detail = Marshal.AllocHGlobal((int)needed);
                        try {
                            Marshal.WriteInt32(detail, IntPtr.Size == 8 ? 8 : 6);
                            if (!SetupDiGetDeviceInterfaceDetail(info, ref data, detail, needed, out needed, IntPtr.Zero)) throw new Win32Exception(Marshal.GetLastWin32Error());
                            string path = Marshal.PtrToStringUni(IntPtr.Add(detail, 4));
                            try { batteries.Add(ReadBattery(path, index)); }
                            catch (Exception error) { batteries.Add(new { index = index, error = error.Message }); }
                        } finally { Marshal.FreeHGlobal(detail); }
                    }
                } finally { SetupDiDestroyDeviceInfoList(info); }
                string json = new JavaScriptSerializer().Serialize(new { captured_utc = DateTime.UtcNow.ToString("o"), source = "Windows battery class QUERY IOCTLs", batteries = batteries });
                using (var writer = new StreamWriter(file)) { writer.WriteLine(json); writer.Flush(); file.Flush(true); }
                Console.WriteLine(json);
            }
            return 0;
        } catch (Exception error) { Console.Error.WriteLine(error.Message); return 1; }
    }
}
