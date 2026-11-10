using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Net;
using System.Reflection;
using System.Security.Cryptography;
using System.Text;
using System.Text.RegularExpressions;
using System.Threading;
using System.Threading.Tasks;
using System.Web.Script.Serialization;

namespace Bricks.Desktop
{
    internal static class Json
    {
        internal static JavaScriptSerializer Serializer()
        {
            return new JavaScriptSerializer { MaxJsonLength = 2 * 1024 * 1024, RecursionLimit = 48 };
        }
        internal static Dictionary<string, object> Parse(string text)
        {
            var value = Serializer().DeserializeObject(text) as Dictionary<string, object>;
            if (value == null) throw new InvalidDataException("The tool returned an unreadable report.");
            return value;
        }
        internal static object Field(Dictionary<string, object> value, string name)
        {
            object result;
            if (!value.TryGetValue(name, out result)) throw new InvalidDataException("Report is missing " + name + ".");
            return result;
        }
        internal static long Count(Dictionary<string, object> value, string name)
        {
            object field = Field(value, name);
            if (!(field is int) && !(field is long) && !(field is decimal))
                throw new InvalidDataException("Invalid count: " + name);
            decimal number = Convert.ToDecimal(field, CultureInfo.InvariantCulture);
            if (number < 0 || decimal.Truncate(number) != number || number > long.MaxValue)
                throw new InvalidDataException("Invalid count: " + name);
            return (long)number;
        }
        internal static bool Flag(Dictionary<string, object> value, string name)
        {
            object field = Field(value, name);
            if (!(field is bool)) throw new InvalidDataException("Invalid flag: " + name);
            return (bool)field;
        }
    }

    internal sealed class CheckResult
    {
        internal string State;
        internal string Summary;
        internal CheckResult(string state, string summary) { State = state; Summary = summary; }
    }

    internal static class Reports
    {
        internal static CheckResult Recovery(Dictionary<string, object> report)
        {
            if ((string)Json.Field(report, "measurement_class") != "deterministic_recovery_simulation"
                || (string)Json.Field(report, "status") != "PASS") throw new InvalidDataException("Recovery check did not pass.");
            long total = Json.Count(report, "scenarios_total");
            var cases = Json.Field(report, "scenarios") as object[];
            if (total == 0 || total != Json.Count(report, "scenarios_passed") || cases == null || cases.Length != total)
                throw new InvalidDataException("Recovery scenario counts do not match.");
            foreach (object item in cases)
            {
                var scenario = item as Dictionary<string, object>;
                if (scenario == null || !Json.Flag(scenario, "passed") || Json.Count(scenario, "owned_keys_after_cleanup") != 0)
                    throw new InvalidDataException("A recovery scenario left unresolved state.");
            }
            return new CheckResult("PASS", total + " simulated scenarios passed. Recorded keys released after every check.");
        }

        internal static void HostCounters(Dictionary<string, object> host, long expected)
        {
            if (expected <= 0 || Json.Count(host, "received_datagrams") != expected
                || Json.Count(host, "applied_transitions") != expected
                || Json.Count(host, "recording_injector_submissions") != expected)
                throw new InvalidDataException("Connection counters do not match.");
            foreach (string name in new[] { "duplicate_transitions", "unchanged_transitions", "owned_keys_before_cleanup", "owned_keys_after_cleanup", "cleanup_failures" })
                if (Json.Count(host, name) != 0) throw new InvalidDataException("Connection check reported " + name + ".");
        }

        internal static CheckResult Probe(Dictionary<string, object> report, bool local)
        {
            if (report.ContainsKey("probe")) report = (Dictionary<string, object>)report["probe"];
            string kind = (string)Json.Field(report, "measurement_class");
            if ((local && kind != "loopback_quic_synthetic")
                || (!local && kind != "loopback_quic_synthetic" && kind != "two_endpoint_quic_synthetic"))
                throw new InvalidDataException("Unexpected connection measurement class.");
            long samples = Json.Count(report, "samples");
            if (samples != 200) throw new InvalidDataException("Expected 200 acknowledged test transitions.");
            HostCounters((Dictionary<string, object>)Json.Field(report, "host"), samples);
            double medianMs = Json.Count(report, "p50_ns") / 1000000.0;
            return new CheckResult("PASS", string.Format(CultureInfo.InvariantCulture,
                "200 test transitions received. {0:0.000} ms median roundtrip; synthetic input only.", medianMs));
        }

        internal static CheckResult Host(Dictionary<string, object> report)
        {
            HostCounters(report, 200);
            return new CheckResult("PASS", "The other PC delivered all 200 test transitions. No recorded keys remain held.");
        }

        internal static CheckResult Input(Dictionary<string, object> report)
        {
            if ((string)Json.Field(report, "measurement_class") != "local_interactive_fixture")
                throw new InvalidDataException("Unexpected input report.");
            if (Json.Count(report, "submission_or_cleanup_failures") != 0 || Json.Flag(report, "unresolved_possible_test_key_down")
                || Json.Flag(report, "awaiting_marked_receipt_at_exit") || Json.Count(report, "raw_read_errors") != 0
                || Json.Count(report, "latency_sample_drops") != 0)
                throw new InvalidDataException("The input test reported a failure or unresolved state. See details.");
            long submissions = Json.Count(report, "explicit_test_submissions");
            var receipts = Json.Field(report, "submit_to_keydown_receipt_ns") as object[];
            if (receipts == null || receipts.Length != submissions) throw new InvalidDataException("Input receipt counts do not match.");
            if (submissions == 0) return new CheckResult("NOT TESTED", "Window closed without a test. Click Run test inside the input window next time.");
            if (Json.Count(report, "unsupported_keyboard_records") != 0)
                return new CheckResult("REVIEW", "Test keys were received, but some keyboard records were unsupported. See details.");
            return new CheckResult("PASS", submissions + " test presses received. Keyboard/mouse counters are available in details.");
        }
    }

    internal sealed class ToolResult
    {
        internal int ExitCode;
        internal string Output;
        internal string Error;
        internal void RequireSuccess()
        {
            if (ExitCode == unchecked((int)0xc0000135))
                throw new InvalidOperationException("A required Windows library could not load (0xc0000135). Check that the x64 Microsoft Visual C++ v14 Redistributable is installed.");
            if (ExitCode != 0) throw new InvalidOperationException(string.IsNullOrWhiteSpace(Error)
                ? "The check could not finish (exit code " + ExitCode + ")." : Error.Trim());
        }
    }

    internal static class Runner
    {
        // ProcessStartInfo on Framework has no ArgumentList. Quote argv directly
        // using Windows escaping; never route user input through any shell.
        internal static string Quote(string argument)
        {
            var text = new StringBuilder("\"");
            int slashes = 0;
            foreach (char c in argument)
            {
                if (c == '\\') { slashes++; continue; }
                if (c == '"') text.Append('\\', slashes * 2 + 1).Append('"');
                else text.Append('\\', slashes).Append(c);
                slashes = 0;
            }
            return text.Append('\\', slashes * 2).Append('"').ToString();
        }
        private static async Task<string> ReadBounded(StreamReader stream)
        {
            var text = new StringBuilder();
            var buffer = new char[4096];
            int count;
            while ((count = await stream.ReadAsync(buffer, 0, buffer.Length).ConfigureAwait(false)) != 0)
            {
                if (text.Length + count > 1024 * 1024) throw new InvalidDataException("Tool output exceeded the 1 MiB limit.");
                text.Append(buffer, 0, count);
            }
            return text.ToString();
        }
        internal static async Task<ToolResult> Run(string executable, string[] args, CancellationToken cancellation,
            int timeoutMs, bool interactive)
        {
            if (interactive && timeoutMs != 0) throw new ArgumentException("Interactive input tests must close gracefully, without a kill timeout.");
            cancellation.ThrowIfCancellationRequested();
            using (var process = new Process())
            {
                process.StartInfo = new ProcessStartInfo {
                    FileName = executable, Arguments = string.Join(" ", args.Select(Quote)),
                    WorkingDirectory = Path.GetDirectoryName(executable), UseShellExecute = false,
                    CreateNoWindow = true, RedirectStandardOutput = true, RedirectStandardError = true,
                    StandardOutputEncoding = Encoding.UTF8, StandardErrorEncoding = Encoding.UTF8
                };
                process.StartInfo.EnvironmentVariables["LOCALAPPDATA"] = Files.LocalAppData;
                process.Start();
                Task<string> output = ReadBounded(process.StandardOutput);
                Task<string> error = ReadBounded(process.StandardError);
                var watch = Stopwatch.StartNew();
                bool closeRequested = false;
                try
                {
                    while (!process.HasExited)
                    {
                        if (output.IsFaulted || error.IsFaulted)
                        {
                            if (interactive) process.CloseMainWindow();
                            else process.Kill();
                            throw new InvalidDataException("The tool returned too much output. Close its test window if it is still open.");
                        }
                        if (cancellation.IsCancellationRequested)
                        {
                            if (!interactive) { process.Kill(); throw new OperationCanceledException(cancellation); }
                            // WM_CLOSE lets the fixture run its owned-key cleanup.
                            // Never terminate a process which may own real OS input.
                            if (!closeRequested) closeRequested = process.CloseMainWindow();
                        }
                        if (timeoutMs > 0 && watch.ElapsedMilliseconds >= timeoutMs)
                        { process.Kill(); throw new TimeoutException("The check timed out."); }
                        await Task.Delay(50).ConfigureAwait(false);
                    }
                    var result = new ToolResult { ExitCode = process.ExitCode,
                        Output = await output.ConfigureAwait(false), Error = await error.ConfigureAwait(false) };
                    cancellation.ThrowIfCancellationRequested();
                    return result;
                }
                finally
                {
                    if (!interactive && !process.HasExited) process.Kill();
                    // Observe reader failures even after cancellation, without blocking
                    // the UI or leaving unobserved task exceptions behind.
                    Observe(output); Observe(error);
                }
            }
        }
        private static void Observe(Task task)
        {
            task.ContinueWith(t => { var ignored = t.Exception; }, TaskContinuationOptions.OnlyOnFaulted);
        }
    }

    internal sealed class Preferences
    {
        public int SchemaVersion = 1;
        public string Role = "host";
        public string HostAddress = "127.0.0.1";
        public string ClientAddress = "127.0.0.1";
        public int Port = 47990;
        public string HostPeer = "";
        public string ClientPeer = "";
        internal static bool FingerprintValid(string value)
        { return value != null && Regex.IsMatch(value, "\\A[0-9a-f]{64}\\z"); }
        internal void Validate()
        {
            if (SchemaVersion != 1 || (Role != "host" && Role != "client") || Port < 1 || Port > 65535
                || HostAddress == null || ClientAddress == null || HostAddress.Length > 80 || ClientAddress.Length > 80
                || (HostPeer != "" && !FingerprintValid(HostPeer)) || (ClientPeer != "" && !FingerprintValid(ClientPeer)))
                throw new InvalidDataException("Saved UI settings are invalid. They have not been used.");
        }
        internal string Peer { get { return Role == "host" ? HostPeer : ClientPeer; } set { if (Role == "host") HostPeer = value; else ClientPeer = value; } }
    }

    internal static class Files
    {
        // Internal dependency seams let the separate test executable isolate all
        // state and tools. The shipped entry point exposes no test-path flags.
        internal static string LocalAppData = Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData);
        internal static string ToolDirectory = Path.Combine(AppDomain.CurrentDomain.BaseDirectory, "bin");
        internal static string DataDirectory { get { return Path.Combine(LocalAppData, "Bricks", "ui"); } }
        internal static string Tool(string name)
        {
            if (name != "bricks-host.exe" && name != "bricks-client.exe" && name != "fixture.exe") throw new ArgumentException("Unknown Bricks tool.");
            string path = Path.Combine(ToolDirectory, name);
            if (!File.Exists(path)) throw new FileNotFoundException("Missing " + name + ". Keep the complete Bricks folder together.", path);
            return path;
        }
        internal static Preferences LoadPreferences()
        {
            string path = Path.Combine(DataDirectory, "settings.json");
            if (!File.Exists(path)) return new Preferences();
            string text = ReadLimited(path, 32768);
            var fields = Json.Parse(text);
            string[] allowed = { "SchemaVersion", "Role", "HostAddress", "ClientAddress", "Port", "HostPeer", "ClientPeer" };
            if (!fields.ContainsKey("SchemaVersion") || fields.Keys.Any(k => !allowed.Contains(k)))
                throw new InvalidDataException("Saved UI settings have an unknown or missing schema field.");
            var result = Json.Serializer().Deserialize<Preferences>(text);
            result.Validate();
            return result;
        }
        internal static void SavePreferences(Preferences preferences)
        {
            preferences.Validate(); Directory.CreateDirectory(DataDirectory);
            string path = Path.Combine(DataDirectory, "settings.json");
            string temporary = path + "." + Guid.NewGuid().ToString("N") + ".tmp";
            try
            {
                File.WriteAllText(temporary, Json.Serializer().Serialize(preferences), new UTF8Encoding(false));
                if (File.Exists(path)) File.Replace(temporary, path, null); else File.Move(temporary, path);
            }
            finally { if (File.Exists(temporary)) File.Delete(temporary); }
        }
        internal static string ReadLimited(string path, int limit)
        {
            using (var file = File.OpenRead(path))
            {
                if (file.Length > limit) throw new InvalidDataException("The selected file is too large.");
                using (var memory = new MemoryStream())
                {
                    var buffer = new byte[4096]; int count;
                    while ((count = file.Read(buffer, 0, buffer.Length)) != 0)
                    {
                        if (memory.Length + count > limit) throw new InvalidDataException("The selected file is too large.");
                        memory.Write(buffer, 0, count);
                    }
                    memory.Position = 0;
                    using (var reader = new StreamReader(memory, Encoding.UTF8, true)) return reader.ReadToEnd();
                }
            }
        }
        internal static byte[] Certificate(string path)
        {
            using (var file = File.OpenRead(path))
            {
                if (file.Length == 0 || file.Length > 65536) throw new InvalidDataException("Choose a public identity file smaller than 64 KiB.");
                using (var memory = new MemoryStream())
                {
                    var buffer = new byte[4096]; int count;
                    while ((count = file.Read(buffer, 0, buffer.Length)) != 0)
                    {
                        if (memory.Length + count > 65536) throw new InvalidDataException("Public identity is too large.");
                        memory.Write(buffer, 0, count);
                    }
                    return memory.ToArray();
                }
            }
        }
        internal static string Fingerprint(byte[] certificate)
        {
            using (var hash = SHA256.Create()) return string.Concat(hash.ComputeHash(certificate).Select(b => b.ToString("x2", CultureInfo.InvariantCulture)));
        }
        internal static string FormatFingerprint(string value)
        {
            if (!Preferences.FingerprintValid(value)) return value;
            return string.Join("\r\n", Enumerable.Range(0, 4).Select(i => value.Substring(i * 16, 16)));
        }
        internal static string Endpoint(string text, int port)
        {
            IPAddress address;
            if (port < 1 || port > 65535 || !IPAddress.TryParse(text.Trim(), out address)
                || address.Equals(IPAddress.Any) || address.Equals(IPAddress.IPv6Any)
                || address.Equals(IPAddress.Broadcast) || address.IsIPv6Multicast
                || (address.AddressFamily == System.Net.Sockets.AddressFamily.InterNetwork && address.GetAddressBytes()[0] >= 224))
                throw new ArgumentException("Enter a specific LAN, loopback, or private-tunnel IP address (without a port).");
            return address.AddressFamily == System.Net.Sockets.AddressFamily.InterNetworkV6
                ? "[" + address + "]:" + port : address + ":" + port;
        }
        internal static string Configuration(string role, string address, int port)
        {
            if (role != "host" && role != "client") throw new ArgumentException("Unknown role.");
            string endpoint = Endpoint(address, port);
            string name = role + ".toml";
            using (Stream stream = Assembly.GetExecutingAssembly().GetManifestResourceStream(name))
            {
                if (stream == null) throw new InvalidDataException("Missing embedded settings.");
                using (var reader = new StreamReader(stream))
                {
                    string key = role == "host" ? "listen" : "peer";
                    return Regex.Replace(reader.ReadToEnd(), "(?m)^" + key + " = \"[^\"]+\"", key + " = \"" + endpoint + "\"");
                }
            }
        }
        internal static string[] Addresses()
        {
            try
            {
                return System.Net.NetworkInformation.NetworkInterface.GetAllNetworkInterfaces()
                    .Where(n => n.OperationalStatus == System.Net.NetworkInformation.OperationalStatus.Up)
                    .SelectMany(n => n.GetIPProperties().UnicastAddresses)
                    .Select(a => a.Address).Where(a => a.AddressFamily == System.Net.Sockets.AddressFamily.InterNetwork && !IPAddress.IsLoopback(a) && !a.Equals(IPAddress.Any))
                    .OrderBy(a => a.GetAddressBytes()[0] == 169 && a.GetAddressBytes()[1] == 254 ? 1 : 0)
                    .Select(a => a.ToString()).Distinct().Concat(new[] { "127.0.0.1" }).ToArray();
            }
            catch (System.Net.NetworkInformation.NetworkInformationException) { return new[] { "127.0.0.1" }; }
        }
    }

    internal sealed class WorkFolder : IDisposable
    {
        internal readonly string PathName;
        internal WorkFolder()
        {
            PathName = Path.Combine(Files.DataDirectory, "work", Guid.NewGuid().ToString("N"));
            Directory.CreateDirectory(PathName);
        }
        public void Dispose()
        {
            // Delete only files this instance explicitly created. No recursive
            // cleanup, computed-directory walking, or access to identity stores.
            foreach (string name in new[] { "public.der", "peer.der", "session.toml" })
            {
                string file = Path.Combine(PathName, name);
                try { if (File.Exists(file)) File.Delete(file); } catch (IOException) { }
            }
            try { Directory.Delete(PathName, false); } catch (IOException) { }
        }
    }
}
