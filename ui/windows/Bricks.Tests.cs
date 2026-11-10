using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Drawing;
using System.Drawing.Imaging;
using System.IO;
using System.Linq;
using System.Net;
using System.Net.NetworkInformation;
using System.Net.Sockets;
using System.Reflection;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
using System.Windows.Forms;

namespace Bricks.Desktop
{
    internal static class FrontendTests
    {
        private static int checks;
        private static int guiExit = 1;
        private static void Require(bool condition, string message)
        { if (!condition) throw new Exception(message); checks++; }
        private static void Reject(Action action, string message)
        {
            bool rejected = false;
            try { action(); } catch (Exception) { rejected = true; }
            Require(rejected, message);
        }
        private static void RejectAsync(Func<Task> action, string message)
        { Reject(() => action().GetAwaiter().GetResult(), message); }

        [STAThread]
        private static int Main(string[] args)
        {
            Console.OutputEncoding = new UTF8Encoding(false);
            if (args.Length > 0 && args[0] == "--echo") { Console.Write(Json.Serializer().Serialize(args.Skip(1).ToArray())); return 0; }
            if (args.Length > 0 && args[0] == "--streams") { Console.Out.Write(new string('o', 200000)); Console.Error.Write(new string('e', 200000)); return 0; }
            if (args.Length > 0 && args[0] == "--overflow") { Console.Out.Write(new string('x', 1100000)); Thread.Sleep(3000); return 0; }
            if (args.Length > 0 && args[0] == "--sleep") { Thread.Sleep(10000); return 0; }
            if (args.Length > 0 && args[0] == "--failure") { Console.Error.Write("deliberate nonzero child"); return 7; }
            if (args.Length < 1) { Console.Error.WriteLine("Provide workspace path, optionally --gui."); return 1; }
            string workspace = Path.GetFullPath(args[0]);
            Files.LocalAppData = Path.Combine(workspace, "target", "ui-tests", "profiles", Guid.NewGuid().ToString("N"));
            Files.ToolDirectory = Path.Combine(workspace, "dist", "Bricks", "bin");
            Directory.CreateDirectory(Files.LocalAppData);
            try
            {
                if (args.Contains("--gui"))
                {
                    Application.EnableVisualStyles(); Application.SetCompatibleTextRenderingDefault(false);
                    Application.SetUnhandledExceptionMode(UnhandledExceptionMode.CatchException);
                    RunGui(workspace); return guiExit;
                }
                RunUnits(workspace);
                Console.WriteLine("PASS: " + checks + " UI controller assertions");
                Directory.CreateDirectory(Path.Combine(workspace, "artifacts"));
                File.WriteAllText(Path.Combine(workspace, "artifacts", "ui-unit-results.json"), Json.Serializer().Serialize(
                    new Dictionary<string, object> { { "status", "PASS" }, { "assertions", checks }, { "class", "windows_ui_controller_tests" } }));
                return 0;
            }
            catch (Exception error) { Console.Error.WriteLine(error); return 1; }
            finally
            {
                // Only remove this test instance's canonical GUID-named profile,
                // strictly beneath workspace/target/ui-tests/profiles.
                string allowed = Path.GetFullPath(Path.Combine(workspace, "target", "ui-tests", "profiles")) + Path.DirectorySeparatorChar;
                string target = Path.GetFullPath(Files.LocalAppData);
                Guid parsed;
                if (target.StartsWith(allowed, StringComparison.OrdinalIgnoreCase)
                    && Guid.TryParseExact(Path.GetFileName(target), "N", out parsed))
                    Directory.Delete(target, true);
            }
        }

        private static void RunUnits(string workspace)
        {
            string child = Assembly.GetExecutingAssembly().Location;
            var arguments = new[] { "", "plain", "white space", "C:\\folder with space\\", "a\\\"b", "\"quotes\"", "雪", "$(`nothing`)&|<>;%PATH%", "ends\\\\" };
            ToolResult echo = Runner.Run(child, new[] { "--echo" }.Concat(arguments).ToArray(), CancellationToken.None, 5000, false).GetAwaiter().GetResult();
            echo.RequireSuccess();
            var actual = (object[])Json.Serializer().DeserializeObject(echo.Output);
            Require(actual.Cast<string>().SequenceEqual(arguments), "Windows argv escaping changed a value.");
            ToolResult streams = Runner.Run(child, new[] { "--streams" }, CancellationToken.None, 5000, false).GetAwaiter().GetResult();
            Require(streams.Output.Length == 200000 && streams.Error.Length == 200000, "Concurrent stdout/stderr drains lost data or deadlocked.");
            ToolResult failure = Runner.Run(child, new[] { "--failure" }, CancellationToken.None, 5000, false).GetAwaiter().GetResult();
            Require(failure.ExitCode == 7, "Child exit code was lost.");
            Reject(failure.RequireSuccess, "Nonzero child was reported as successful.");
            Reject(new ToolResult { ExitCode = unchecked((int)0xc0000135) }.RequireSuccess, "Missing runtime was reported as successful.");
            RejectAsync(() => Runner.Run(child, new[] { "--overflow" }, CancellationToken.None, 5000, false), "Output cap not enforced.");
            RejectAsync(() => Runner.Run(child, new[] { "--sleep" }, CancellationToken.None, 100, false), "Timeout not enforced.");
            using (var cancel = new CancellationTokenSource()) {
                cancel.CancelAfter(100);
                RejectAsync(() => Runner.Run(child, new[] { "--sleep" }, cancel.Token, 5000, false), "Cancellation not enforced.");
            }
            RejectAsync(() => Runner.Run(child, new string[0], CancellationToken.None, 100, true), "Unsafe interactive kill timeout was accepted.");

            Require(Files.Endpoint("192.168.1.5", 47990) == "192.168.1.5:47990", "IPv4 endpoint failed.");
            Require(Files.Endpoint("::1", 1234) == "[::1]:1234", "IPv6 endpoint failed.");
            Require(Files.Endpoint("100.64.1.2", 47990) == "100.64.1.2:47990", "Tunnel endpoint failed.");
            foreach (string invalid in new[] { "0.0.0.0", "::", "224.0.0.1", "ff02::1", "255.255.255.255", "host; echo bad", "127.0.0.1:47990" })
                Reject(() => Files.Endpoint(invalid, 47990), "Invalid address accepted: " + invalid);
            Reject(() => Files.Endpoint("127.0.0.1", 0), "Port zero accepted.");
            Reject(() => Files.Tool("cmd.exe"), "Unexpected tool executable accepted.");

            var preferences = new Preferences { Role = "client", HostAddress = "192.168.1.5", ClientAddress = "100.64.1.2", Port = 43210, HostPeer = new string('a', 64), ClientPeer = new string('b', 64) };
            Files.SavePreferences(preferences);
            Preferences loaded = Files.LoadPreferences();
            Require(loaded.Role == "client" && loaded.Peer == new string('b', 64) && loaded.Port == 43210, "Saved public settings did not roundtrip.");
            Require(loaded.HostAddress == "192.168.1.5" && loaded.ClientAddress == "100.64.1.2", "Role-specific addresses were lost.");
            string file = Path.Combine(Files.DataDirectory, "settings.json");
            File.WriteAllText(file, "{\"SchemaVersion\":2}"); Reject(() => Files.LoadPreferences(), "Future UI schema accepted.");
            File.WriteAllText(file, "{}"); Reject(() => Files.LoadPreferences(), "Missing UI schema accepted.");
            File.WriteAllText(file, "{\"SchemaVersion\":1,\"unknown\":true}"); Reject(() => Files.LoadPreferences(), "Unknown UI field accepted.");
            File.WriteAllText(file, "{\"SchemaVersion\":1,\"HostPeer\":null}"); Reject(() => Files.LoadPreferences(), "Invalid saved peer accepted.");
            string oversized = Path.Combine(Files.LocalAppData, "large.der");
            File.WriteAllBytes(oversized, new byte[65537]); Reject(() => Files.Certificate(oversized), "Oversized public certificate accepted.");
            Require(Files.Fingerprint(new byte[] { 1, 2, 3 }).Length == 64, "Fingerprint malformed.");
            string fingerprint = new string('a', 64);
            Require(Files.FormatFingerprint(fingerprint).Replace("\r\n", "") == fingerprint, "Display changed fingerprint.");
            using (var work = new WorkFolder()) {
                File.WriteAllText(Path.Combine(work.PathName, "session.toml"), "test");
                string ownPath = work.PathName; work.Dispose(); Require(!Directory.Exists(ownPath), "Owned work files not cleaned.");
            }

            ToolResult recoveryRun = Runner.Run(Files.Tool("bricks-host.exe"), new[] { "recovery-selftest" }, CancellationToken.None, 10000, false).GetAwaiter().GetResult();
            recoveryRun.RequireSuccess();
            string recoveryText = recoveryRun.Output;
            var recovery = Json.Parse(recoveryText); Require(Reports.Recovery(recovery).State == "PASS", "Actual recovery report rejected.");
            recovery["scenarios_passed"] = 0; Reject(() => Reports.Recovery(recovery), "Incomplete recovery accepted.");
            recovery = Json.Parse(recoveryText);
            ((Dictionary<string, object>)((object[])recovery["scenarios"])[0])["owned_keys_after_cleanup"] = 1;
            Reject(() => Reports.Recovery(recovery), "Unresolved recorded key accepted.");
            var probe = new Dictionary<string, object> {
                { "measurement_class", "loopback_quic_synthetic" }, { "samples", 200 }, { "p50_ns", 400000 },
                { "host", new Dictionary<string, object> {
                    { "received_datagrams", 200 }, { "applied_transitions", 200 }, { "recording_injector_submissions", 200 },
                    { "duplicate_transitions", 0 }, { "unchanged_transitions", 0 }, { "owned_keys_before_cleanup", 0 }, { "owned_keys_after_cleanup", 0 }, { "cleanup_failures", 0 }
                } }
            };
            Require(Reports.Probe(probe, true).State == "PASS", "Valid probe rejected.");
            ((Dictionary<string, object>)probe["host"])["cleanup_failures"] = 1;
            Reject(() => Reports.Probe(probe, true), "Probe cleanup failure accepted.");
            var input = new Dictionary<string, object> {
                { "measurement_class", "local_interactive_fixture" }, { "submission_or_cleanup_failures", 0 },
                { "unresolved_possible_test_key_down", false }, { "awaiting_marked_receipt_at_exit", false }, { "raw_read_errors", 0 },
                { "latency_sample_drops", 0 }, { "explicit_test_submissions", 0 }, { "submit_to_keydown_receipt_ns", new object[0] }, { "unsupported_keyboard_records", 0 }
            };
            Require(Reports.Input(input).State == "NOT TESTED", "No submissions were mislabeled PASS.");
            input["explicit_test_submissions"] = 1; input["submit_to_keydown_receipt_ns"] = new object[] { 500000 };
            Require(Reports.Input(input).State == "PASS", "Valid fixture rejected.");
            input["unresolved_possible_test_key_down"] = true;
            Reject(() => Reports.Input(input), "Unresolved real-input state accepted.");
            Reject(() => Json.Count(new Dictionary<string, object> { { "count", 1.5 } }, "count"), "Fractional count accepted.");
            Reject(() => Json.Count(new Dictionary<string, object> { { "count", -1 } }, "count"), "Negative count accepted.");

            foreach (string role in new[] { "host", "client" }) {
                string config = Path.Combine(Files.LocalAppData, role + ".toml");
                File.WriteAllText(config, Files.Configuration(role, "::1", 43210), new UTF8Encoding(false));
                ToolResult result = Runner.Run(Files.Tool("bricks-" + role + ".exe"), new[] { "config", config }, CancellationToken.None, 5000, false).GetAwaiter().GetResult();
                result.RequireSuccess(); Require(result.Output.Contains("[::1]:43210"), "Rust did not validate the generated role configuration.");
            }
        }

        private static T Find<T>(Control parent, string name) where T : Control
        { return (T)parent.Controls.Find(name, true).Single(); }
        private static async Task Until(Func<bool> ready, string message, int timeout)
        {
            var watch = Stopwatch.StartNew();
            while (!ready()) {
                if (watch.ElapsedMilliseconds > timeout) throw new Exception(message);
                await Task.Delay(40);
            }
        }
        private static void Screenshot(Form form, string path)
        {
            form.PerformLayout(); form.Refresh();
            using (var image = new Bitmap(form.Width, form.Height)) {
                form.DrawToBitmap(image, new Rectangle(0, 0, image.Width, image.Height)); image.Save(path, ImageFormat.Png);
            }
        }
        private static int AvailablePort()
        { using (var socket = new UdpClient(new IPEndPoint(IPAddress.Loopback, 0))) return ((IPEndPoint)socket.Client.LocalEndPoint).Port; }
        private static bool Listening(int port)
        { return IPGlobalProperties.GetIPGlobalProperties().GetActiveUdpListeners().Any(e => e.Port == port && e.Address.Equals(IPAddress.Loopback)); }
        private static async Task<ToolResult> Identity(string role, params string[] values)
        {
            var result = await Runner.Run(Files.Tool("bricks-" + role + ".exe"), values, CancellationToken.None, 10000, false);
            result.RequireSuccess(); return result;
        }
        private static async Task<string> NewIdentity(string role)
        {
            ToolResult result = await Identity(role, "identity", "init", "os-store:bricks-" + role);
            string fingerprint = result.Output.Trim().Substring("identity_sha256=".Length);
            Require(Preferences.FingerprintValid(fingerprint), "Generated test identity fingerprint is invalid.");
            return fingerprint;
        }
        private static void RunGui(string workspace)
        {
            // Only identities generated in this owned test root are pretrusted.
            // No real user identity, public certificate or saved pairing is used.
            string host = NewIdentity("host").GetAwaiter().GetResult();
            string client = NewIdentity("client").GetAwaiter().GetResult();
            string hostPublic = Path.Combine(Files.LocalAppData, "host.der");
            string clientPublic = Path.Combine(Files.LocalAppData, "client.der");
            Identity("host", "identity", "export", "os-store:bricks-host", hostPublic).GetAwaiter().GetResult();
            Identity("client", "identity", "export", "os-store:bricks-client", clientPublic).GetAwaiter().GetResult();
            Identity("host", "identity", "trust", "os-store:bricks-host", clientPublic, client, "--allow-control").GetAwaiter().GetResult();
            Identity("client", "identity", "trust", "os-store:bricks-client", hostPublic, host).GetAwaiter().GetResult();
            Files.SavePreferences(new Preferences { HostPeer = client, ClientPeer = host, Port = AvailablePort() });
            string artifacts = Path.Combine(workspace, "artifacts"); Directory.CreateDirectory(artifacts);
            using (var form = new BricksForm())
            {
                form.Shown += async delegate {
                    var pulse = new System.Windows.Forms.Timer { Interval = 20 }; int ticks = 0;
                    pulse.Tick += delegate { ticks++; }; pulse.Start();
                    try {
                        await Task.Delay(150);
                        Screenshot(form, Path.Combine(artifacts, "ui-home.png"));
                        Find<Button>(form, "HomeRunChecksButton").PerformClick();
                        await Until(() => !Find<Button>(form, "CancelCheckButton").Enabled, "Automatic checks did not finish.", 25000);
                        Require(Find<Label>(form, "RecoveryStatus").Text == "PASS", "Recovery UI did not display PASS.");
                        Require(Find<Label>(form, "LoopbackStatus").Text == "PASS", "Local connection UI did not display PASS.");
                        Require(ticks > 2, "UI message loop did not remain responsive.");
                        Screenshot(form, Path.Combine(artifacts, "ui-checks.png"));

                        // Cancel requests WM_CLOSE on this owned fixture process;
                        // never synthesize F8, mouse input or an OS key press.
                        Find<Button>(form, "InputButton").PerformClick(); await Task.Delay(350);
                        Find<Button>(form, "CancelCheckButton").PerformClick();
                        await Until(() => !Find<Button>(form, "CancelCheckButton").Enabled, "Fixture did not close gracefully.", 6000);
                        Require(Find<Label>(form, "InputStatus").Text == "CANCELLED", "Fixture cancellation status missing.");

                        Find<Button>(form, "NavConnection").PerformClick();
                        Find<Button>(form, "PrepareDeviceButton").PerformClick();
                        await Until(() => !Find<Button>(form, "CancelCheckButton").Enabled, "Host setup did not finish.", 6000);
                        Require(Find<TextBox>(form, "OwnFingerprint").Text.Replace("\r\n", "") == host, "Host identity display is incorrect.");
                        Screenshot(form, Path.Combine(artifacts, "ui-connection.png"));
                        int port = AvailablePort();
                        Find<ComboBox>(form, "ConnectionAddress").Text = "127.0.0.1";
                        Find<NumericUpDown>(form, "ConnectionPort").Value = port;
                        Find<Button>(form, "StartConnectionButton").PerformClick();
                        await Until(() => Listening(port), "GUI host did not start listening.", 6000);
                        string config = Path.Combine(Files.LocalAppData, "client.toml");
                        File.WriteAllText(config, Files.Configuration("client", "127.0.0.1", port), new UTF8Encoding(false));
                        ToolResult clientResult = await Runner.Run(Files.Tool("bricks-client.exe"), new[] { "probe-client", config, host, "100" }, CancellationToken.None, 10000, false);
                        clientResult.RequireSuccess();
                        await Until(() => !Find<Button>(form, "CancelCheckButton").Enabled, "GUI host did not finish.", 6000);
                        Require(Find<Label>(form, "ConnectionStatus").Text.Contains("all 200"), "GUI host did not report successful connection.");

                        Find<ComboBox>(form, "DeviceRole").SelectedIndex = 1;
                        Require(!Find<Button>(form, "StartConnectionButton").Enabled, "Role switch reused unprepared identity state.");
                        Find<Button>(form, "PrepareDeviceButton").PerformClick();
                        await Until(() => !Find<Button>(form, "CancelCheckButton").Enabled, "Client setup did not finish.", 6000);
                        Require(Find<TextBox>(form, "OwnFingerprint").Text.Replace("\r\n", "") == client, "Client identity display is incorrect.");
                        port = AvailablePort(); Find<ComboBox>(form, "ConnectionAddress").Text = "127.0.0.1";
                        Find<NumericUpDown>(form, "ConnectionPort").Value = port;
                        config = Path.Combine(Files.LocalAppData, "host.toml");
                        File.WriteAllText(config, Files.Configuration("host", "127.0.0.1", port), new UTF8Encoding(false));
                        Task<ToolResult> server = Runner.Run(Files.Tool("bricks-host.exe"), new[] { "probe-host", config, client, "--approve-control" }, CancellationToken.None, 10000, false);
                        await Until(() => Listening(port), "Test server did not listen.", 6000);
                        Find<Button>(form, "StartConnectionButton").PerformClick();
                        (await server).RequireSuccess();
                        await Until(() => !Find<Button>(form, "CancelCheckButton").Enabled, "GUI client did not finish.", 6000);
                        Require(Find<Label>(form, "ConnectionStatus").Text.Contains("200 test transitions"), "GUI client did not report successful connection.");

                        Find<ComboBox>(form, "DeviceRole").SelectedIndex = 0;
                        Find<Button>(form, "PrepareDeviceButton").PerformClick();
                        await Until(() => !Find<Button>(form, "CancelCheckButton").Enabled, "Reloading host failed.", 6000);
                        port = AvailablePort(); Find<ComboBox>(form, "ConnectionAddress").Text = "127.0.0.1";
                        Find<NumericUpDown>(form, "ConnectionPort").Value = port;
                        Find<Button>(form, "StartConnectionButton").PerformClick();
                        await Until(() => Listening(port), "Cancel-test server did not listen.", 6000);
                        Find<Button>(form, "CancelCheckButton").PerformClick();
                        await Until(() => !Find<Button>(form, "CancelCheckButton").Enabled, "Host cancellation did not finish.", 6000);
                        Require(!Listening(port), "Cancelled host left a listening socket.");
                        Require(Find<Label>(form, "ConnectionStatus").Text.Contains("cancelled"), "Cancellation did not update the connection UI.");
                        guiExit = 0;
                        Console.WriteLine("PASS: " + checks + " GUI integration assertions (recording-only loopback; no OS input injected)");
                        File.WriteAllText(Path.Combine(artifacts, "ui-integration-results.json"), Json.Serializer().Serialize(
                            new Dictionary<string, object> { { "status", "PASS" }, { "assertions", checks }, { "class", "windows_ui_recording_loopback_integration" }, { "os_input_injected", false } }));
                    }
                    catch (Exception error) { Console.Error.WriteLine(error); }
                    finally { pulse.Stop(); pulse.Dispose(); form.Close(); }
                };
                Application.Run(form);
            }
        }
    }
}
