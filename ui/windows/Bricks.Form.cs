using System;
using System.Collections.Generic;
using System.Drawing;
using System.Drawing.Drawing2D;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Text;
using System.Text.RegularExpressions;
using System.Threading;
using System.Threading.Tasks;
using System.Windows.Forms;

namespace Bricks.Desktop
{
    internal static class Palette
    {
        internal static readonly Color Ink = Color.FromArgb(24, 43, 48);
        internal static readonly Color Muted = Color.FromArgb(89, 108, 113);
        internal static readonly Color Teal = Color.FromArgb(0, 112, 100);
        internal static readonly Color Paper = Color.FromArgb(245, 247, 245);
        internal static readonly Color Line = Color.FromArgb(216, 226, 222);
        internal static readonly Color Navy = Color.FromArgb(18, 37, 44);
        internal static readonly Color Red = Color.FromArgb(162, 48, 42);
    }

    internal sealed class Brand : Control
    {
        internal Brand() { Height = 86; Dock = DockStyle.Top; DoubleBuffered = true; }
        protected override void OnPaint(PaintEventArgs e)
        {
            base.OnPaint(e);
            float scale = e.Graphics.DpiX / 96f;
            e.Graphics.ScaleTransform(scale, scale);
            using (var brush = new SolidBrush(Color.FromArgb(100, 222, 182)))
            {
                e.Graphics.FillRectangle(brush, 0, 8, 16, 8);
                e.Graphics.FillRectangle(brush, 19, 8, 16, 8);
                e.Graphics.FillRectangle(brush, 0, 19, 25, 8);
                e.Graphics.FillRectangle(brush, 28, 19, 7, 8);
            }
            using (var font = new Font("Segoe UI", 24, FontStyle.Bold, GraphicsUnit.Pixel))
            using (var brush = new SolidBrush(Color.White)) e.Graphics.DrawString("bricks", font, brush, 44, 0);
            using (var font = new Font("Segoe UI", 12, FontStyle.Regular, GraphicsUnit.Pixel))
            using (var brush = new SolidBrush(Color.FromArgb(166, 190, 188)))
                e.Graphics.DrawString("WINDOWS PREVIEW", font, brush, 0, 48);
        }
    }

    internal class Card : Panel
    {
        internal Card()
        {
            BackColor = Color.White; Padding = new Padding(20); Margin = new Padding(0, 0, 0, 16);
            DoubleBuffered = true;
        }
        protected override void OnPaint(PaintEventArgs e)
        {
            base.OnPaint(e);
            using (var pen = new Pen(Palette.Line)) e.Graphics.DrawRectangle(pen, 0, 0, Width - 1, Height - 1);
        }
    }

    internal sealed class CheckCard : Card
    {
        internal readonly Label Badge;
        internal readonly Label Summary;
        internal readonly Button Action;
        internal CheckCard(string title, string description, string actionName, string accessibilityName)
        {
            Height = 130; Dock = DockStyle.Top;
            var grid = new TableLayoutPanel { Dock = DockStyle.Fill, ColumnCount = 2, RowCount = 3, BackColor = Color.Transparent };
            grid.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100)); grid.ColumnStyles.Add(new ColumnStyle(SizeType.Absolute, 126));
            grid.RowStyles.Add(new RowStyle(SizeType.Absolute, 30)); grid.RowStyles.Add(new RowStyle(SizeType.Absolute, 28)); grid.RowStyles.Add(new RowStyle(SizeType.Percent, 100));
            grid.Controls.Add(BricksForm.Label(title, 13, FontStyle.Bold), 0, 0);
            Summary = BricksForm.Label(description, 9, FontStyle.Regular); Summary.Name = accessibilityName + "Summary";
            grid.Controls.Add(Summary, 0, 1); grid.SetRowSpan(Summary, 2);
            Badge = BricksForm.Label("NOT RUN", 9, FontStyle.Bold); Badge.Name = accessibilityName + "Status";
            Badge.TextAlign = ContentAlignment.TopRight; grid.Controls.Add(Badge, 1, 0);
            Action = BricksForm.Button(actionName, false); Action.Name = accessibilityName + "Button";
            Action.Dock = DockStyle.Fill; grid.Controls.Add(Action, 1, 1); grid.SetRowSpan(Action, 2);
            Controls.Add(grid);
        }
        internal void Set(string state, string summary)
        {
            Badge.Text = state; Summary.Text = summary;
            Badge.ForeColor = state == "PASS" ? Palette.Teal : state == "FAILED" ? Palette.Red : Palette.Muted;
        }
    }

    internal sealed class BricksForm : Form
    {
        private readonly Dictionary<string, Panel> pages = new Dictionary<string, Panel>();
        private readonly Dictionary<string, Button> navigation = new Dictionary<string, Button>();
        private readonly List<Button> actions = new List<Button>();
        private readonly Panel body = new Panel { Dock = DockStyle.Fill, BackColor = Palette.Paper };
        private readonly Label footer;
        private readonly ProgressBar progress;
        private readonly Button cancelButton;
        private readonly Button exportButton;
        private CheckCard recoveryCard, networkCard, inputCard;
        private TextBox details, fingerprint;
        private ComboBox role, address;
        private NumericUpDown port;
        private Label peerLabel, connectionResult;
        private Button prepareButton, saveIdentityButton, trustButton, forgetButton, connectionButton;
        private Preferences preferences;
        private bool busy, loading, closeWhenFinished;
        private string ownFingerprint = "";
        private CheckCard runningCard;
        private string currentPage = "home";
        private CancellationTokenSource cancellation;
        private Dictionary<string, object> lastReport;

        internal BricksForm()
        {
            Text = "Bricks - Windows preview"; Name = "BricksWindow";
            Icon = Icon.ExtractAssociatedIcon(Application.ExecutablePath);
            Font = new Font("Segoe UI", 10); ForeColor = Palette.Ink; BackColor = Palette.Paper;
            AutoScaleDimensions = new SizeF(96, 96); AutoScaleMode = AutoScaleMode.Dpi;
            Size = new Size(1130, 850); MinimumSize = new Size(880, 600); StartPosition = FormStartPosition.CenterScreen;
            string settingsError = null;
            try { preferences = Files.LoadPreferences(); }
            catch (Exception error) { preferences = new Preferences(); settingsError = error.Message; }

            var sidebar = new Panel { Dock = DockStyle.Left, Width = 215, BackColor = Palette.Navy, Padding = new Padding(20, 28, 20, 20) };
            var nav = new FlowLayoutPanel { Dock = DockStyle.Fill, FlowDirection = FlowDirection.TopDown, WrapContents = false, Padding = new Padding(0, 18, 0, 0) };
            AddNavigation(nav, "Home", "home"); AddNavigation(nav, "Checks", "checks"); AddNavigation(nav, "Connection", "connection");
            var sideBottom = new Panel { Dock = DockStyle.Bottom, Height = 200 };
            var preview = Label("INPUT-ONLY PREVIEW\r\n\r\nDesktop control and video\r\nare still in development.", 9, FontStyle.Regular);
            preview.ForeColor = Color.FromArgb(170, 195, 193); preview.Dock = DockStyle.Top; preview.Height = 112;
            exportButton = Button("Export last report", false); exportButton.Dock = DockStyle.Bottom;
            exportButton.Name = "ExportReportButton"; exportButton.Click += delegate { ExportReport(); };
            sideBottom.Controls.Add(preview); sideBottom.Controls.Add(exportButton);
            sidebar.Controls.Add(nav); sidebar.Controls.Add(new Brand()); sidebar.Controls.Add(sideBottom);

            var statusBar = new Panel { Dock = DockStyle.Bottom, Height = 60, BackColor = Color.White, Padding = new Padding(24, 10, 18, 10) };
            cancelButton = Button("Cancel check", false); cancelButton.Name = "CancelCheckButton";
            cancelButton.Dock = DockStyle.Right; cancelButton.Width = 140;
            cancelButton.Click += delegate { if (cancellation != null) { cancellation.Cancel(); SetFooter("Stopping the check and closing its test window..."); } };
            footer = Label(settingsError ?? "Ready. No active connection.", 9, FontStyle.Regular); footer.Name = "ActivityStatus"; footer.Dock = DockStyle.Fill;
            statusBar.Controls.Add(footer); statusBar.Controls.Add(cancelButton);
            progress = new ProgressBar { Dock = DockStyle.Bottom, Height = 3, Style = ProgressBarStyle.Marquee, MarqueeAnimationSpeed = 25, Visible = false };
            Controls.Add(body); Controls.Add(progress); Controls.Add(statusBar); Controls.Add(sidebar);
            BuildHome(); BuildChecks(); BuildConnection(); Navigate("home"); UpdateEnabled();
            Shown += delegate {
                Rectangle area = Screen.FromControl(this).WorkingArea;
                Size = new Size(Math.Min(Width, area.Width - 24), Math.Min(Height, area.Height - 24));
                CenterToScreen();
            };
            FormClosing += delegate(object sender, FormClosingEventArgs e) {
                if (!busy) return;
                e.Cancel = true; closeWhenFinished = true;
                cancellation.Cancel(); SetFooter("Finishing cleanup before closing...");
            };
        }

        internal static Label Label(string text, float size, FontStyle style)
        {
            return new Label { Text = text, Font = new Font("Segoe UI", size, style), Dock = DockStyle.Fill,
                ForeColor = style == FontStyle.Bold ? Palette.Ink : Palette.Muted, Margin = new Padding(0), AutoEllipsis = true };
        }
        internal static Button Button(string text, bool primary)
        {
            var button = new Button { Text = text, AutoSize = false, Height = 42, Width = 210, FlatStyle = FlatStyle.Flat,
                BackColor = primary ? Palette.Teal : Color.White, ForeColor = primary ? Color.White : Palette.Ink,
                Font = new Font("Segoe UI", 10, FontStyle.Bold), Cursor = Cursors.Hand, UseVisualStyleBackColor = false,
                Margin = new Padding(0, 0, 10, 0) };
            button.FlatAppearance.BorderSize = primary ? 0 : 1; button.FlatAppearance.BorderColor = Palette.Line;
            return button;
        }
        private void AddNavigation(FlowLayoutPanel parent, string text, string key)
        {
            Button button = Button(text, false); button.Width = 175; button.Height = 46; button.Margin = new Padding(0, 0, 0, 9);
            button.TextAlign = ContentAlignment.MiddleLeft; button.Padding = new Padding(12, 0, 0, 0); button.FlatAppearance.BorderSize = 0;
            button.Name = "Nav" + text; button.Click += delegate { Navigate(key); };
            navigation[key] = button; parent.Controls.Add(button);
        }
        private Panel NewPage(string key)
        {
            var page = new Panel { Dock = DockStyle.Fill, AutoScroll = true, Padding = new Padding(28), BackColor = Palette.Paper };
            page.Name = key + "Page"; pages[key] = page; body.Controls.Add(page); return page;
        }
        private TableLayoutPanel Stack(Panel parent, params int[] heights)
        {
            var stack = new TableLayoutPanel { Dock = DockStyle.Top, AutoSize = true, ColumnCount = 1, RowCount = heights.Length, Margin = Padding.Empty };
            stack.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100));
            foreach (int height in heights) stack.RowStyles.Add(new RowStyle(SizeType.Absolute, height));
            parent.Controls.Add(stack); return stack;
        }
        private void Navigate(string page)
        {
            currentPage = page;
            foreach (var pair in pages) pair.Value.Visible = pair.Key == page;
            pages[page].BringToFront();
            pages[page].AutoScrollPosition = Point.Empty;
            foreach (var pair in navigation) {
                pair.Value.BackColor = pair.Key == page ? Color.FromArgb(38, 68, 73) : Palette.Navy;
                pair.Value.ForeColor = pair.Key == page ? Color.FromArgb(143, 242, 207) : Color.FromArgb(203, 218, 217);
            }
        }
        private Button ActionButton(string text, string name, bool primary)
        { Button button = Button(text, primary); button.Name = name; actions.Add(button); return button; }

        private void BuildHome()
        {
            Panel page = NewPage("home"); var stack = Stack(page, 28, 99, 68, 65, 36, 148, 148, 52);
            var eyebrow = Label("A LITTLE CLOSER TO INSTANT.", 9, FontStyle.Bold); eyebrow.ForeColor = Palette.Teal; stack.Controls.Add(eyebrow, 0, 0);
            stack.Controls.Add(Label("Ready when\r\nyou are.", 30, FontStyle.Bold), 0, 1);
            stack.Controls.Add(Label("Check this PC today. Connect your second PC when it is ready.\r\nYour tests, setup and results now live in one place.", 11, FontStyle.Regular), 0, 2);
            var start = ActionButton("Run automatic checks", "HomeRunChecksButton", true); start.Width = 250;
            start.Click += async delegate { Navigate("checks"); await RunAutomatic(); }; stack.Controls.Add(start, 0, 3);
            stack.Controls.Add(Label("YOUR NEXT STEPS", 9, FontStyle.Bold), 0, 4);
            var local = new CheckCard("01   Check your input", "Open the input window, then click Run test. No Fn key needed.", "Open test", "HomeInput");
            local.Badge.Text = "THIS PC"; local.Dock = DockStyle.Fill; local.Margin = new Padding(0, 0, 0, 14);
            local.Action.Click += async delegate { Navigate("checks"); await RunInput(); }; actions.Add(local.Action); stack.Controls.Add(local, 0, 5);
            var connection = new CheckCard("02   Get ready for another PC", "Set up a device identity, verify the other PC, then test the connection.", "Set up", "HomeConnection");
            connection.Badge.Text = "WHEN READY"; connection.Dock = DockStyle.Fill; connection.Margin = new Padding(0, 0, 0, 14);
            connection.Action.Click += delegate { Navigate("connection"); }; stack.Controls.Add(connection, 0, 6);
            stack.Controls.Add(Label("Windows to Windows  /  LAN or your existing private tunnel", 9, FontStyle.Regular), 0, 7);
        }

        private void BuildChecks()
        {
            Panel page = NewPage("checks"); var stack = Stack(page, 47, 53, 61, 144, 144, 144, 30, 170);
            stack.Controls.Add(Label("Give the basics a check.", 23, FontStyle.Bold), 0, 0);
            stack.Controls.Add(Label("Automatic checks need only this PC. The input test opens a separate window.\r\nResults here are diagnostics, not gaming latency benchmarks.", 10, FontStyle.Regular), 0, 1);
            var all = ActionButton("Run automatic checks", "RunChecksButton", true); all.Width = 250;
            all.Click += async delegate { await RunAutomatic(); }; stack.Controls.Add(all, 0, 2);
            recoveryCard = new CheckCard("Input recovery", "Simulate lost packets, ordering, retries and recorded-key cleanup.", "Run check", "Recovery");
            networkCard = new CheckCard("Local connection", "Verify an authenticated connection through this PC's network stack.", "Run check", "Loopback");
            inputCard = new CheckCard("Keyboard & mouse", "Open the test window. Click Run test, then close it for your results.", "Open test", "Input");
            recoveryCard.Dock = networkCard.Dock = inputCard.Dock = DockStyle.Fill;
            recoveryCard.Action.Click += async delegate { await RunSingle(false); };
            networkCard.Action.Click += async delegate { await RunSingle(true); };
            inputCard.Action.Click += async delegate { await RunInput(); };
            actions.Add(recoveryCard.Action); actions.Add(networkCard.Action); actions.Add(inputCard.Action);
            stack.Controls.Add(recoveryCard, 0, 3); stack.Controls.Add(networkCard, 0, 4); stack.Controls.Add(inputCard, 0, 5);
            stack.Controls.Add(Label("LAST REPORT / DETAILS", 9, FontStyle.Bold), 0, 6);
            details = new TextBox { Multiline = true, ReadOnly = true, ScrollBars = ScrollBars.Both, WordWrap = false,
                Dock = DockStyle.Fill, Font = new Font("Consolas", 9), BackColor = Color.White, BorderStyle = BorderStyle.FixedSingle,
                Text = "Run a check to see its report here.", Name = "ReportDetails" };
            stack.Controls.Add(details, 0, 7);
        }

        private void BuildConnection()
        {
            Panel page = NewPage("connection"); var stack = Stack(page, 47, 57, 75, 236, 165, 179, 85);
            stack.Controls.Add(Label("A connection, made clearer.", 23, FontStyle.Bold), 0, 0);
            stack.Controls.Add(Label("Use this screen on both PCs. This preview tests the connection only.\r\nIt does not control the desktop or stream video.", 10, FontStyle.Regular), 0, 1);
            var roleRow = new FlowLayoutPanel { Dock = DockStyle.Fill, WrapContents = false, Margin = Padding.Empty };
            roleRow.Controls.Add(new Label { Text = "THIS PC", Width = 86, Height = 38, TextAlign = ContentAlignment.MiddleLeft, ForeColor = Palette.Muted });
            role = new ComboBox { DropDownStyle = ComboBoxStyle.DropDownList, Width = 380, Name = "DeviceRole" };
            role.Items.AddRange(new object[] { "Host - the PC you want to control", "Client - the PC you will use" });
            role.SelectedIndex = preferences.Role == "host" ? 0 : 1;
            role.SelectedIndexChanged += delegate {
                if (loading) return; SaveAddress(); preferences.Role = role.SelectedIndex == 0 ? "host" : "client";
                ownFingerprint = ""; fingerprint.Text = "Set up this device to show its fingerprint.";
                LoadAddress(); RefreshPeer(); UpdateEnabled();
            };
            roleRow.Controls.Add(role); stack.Controls.Add(roleRow, 0, 2);

            var device = new Card { Dock = DockStyle.Fill }; var deviceGrid = new TableLayoutPanel { Dock = DockStyle.Fill, ColumnCount = 1, RowCount = 4 };
            deviceGrid.RowStyles.Add(new RowStyle(SizeType.Absolute, 30)); deviceGrid.RowStyles.Add(new RowStyle(SizeType.Absolute, 38));
            deviceGrid.RowStyles.Add(new RowStyle(SizeType.Percent, 100)); deviceGrid.RowStyles.Add(new RowStyle(SizeType.Absolute, 45));
            deviceGrid.Controls.Add(Label("1   Your device", 13, FontStyle.Bold), 0, 0);
            deviceGrid.Controls.Add(Label("Create or load this PC's protected identity. Share only its public identity file.", 9, FontStyle.Regular), 0, 1);
            fingerprint = new TextBox { Multiline = true, ReadOnly = true, BorderStyle = BorderStyle.None, BackColor = Color.White,
                Font = new Font("Consolas", 10), Dock = DockStyle.Fill, Name = "OwnFingerprint", Text = "Set up this device to show its fingerprint." };
            deviceGrid.Controls.Add(fingerprint, 0, 2);
            var deviceActions = new FlowLayoutPanel { Dock = DockStyle.Fill, WrapContents = false, Margin = Padding.Empty };
            prepareButton = ActionButton("Set up this device", "PrepareDeviceButton", true);
            saveIdentityButton = ActionButton("Save public identity", "SaveIdentityButton", false);
            prepareButton.Click += async delegate { await PrepareDevice(); }; saveIdentityButton.Click += async delegate { await ExportIdentity(); };
            deviceActions.Controls.Add(prepareButton); deviceActions.Controls.Add(saveIdentityButton); deviceGrid.Controls.Add(deviceActions, 0, 3);
            device.Controls.Add(deviceGrid); stack.Controls.Add(device, 0, 3);

            var peer = new Card { Dock = DockStyle.Fill }; var peerGrid = new TableLayoutPanel { Dock = DockStyle.Fill, ColumnCount = 1, RowCount = 3 };
            peerGrid.RowStyles.Add(new RowStyle(SizeType.Absolute, 30)); peerGrid.RowStyles.Add(new RowStyle(SizeType.Percent, 100)); peerGrid.RowStyles.Add(new RowStyle(SizeType.Absolute, 45));
            peerGrid.Controls.Add(Label("2   Verify the other PC", 13, FontStyle.Bold), 0, 0);
            peerLabel = Label("", 9, FontStyle.Regular); peerLabel.Name = "PeerStatus"; peerGrid.Controls.Add(peerLabel, 0, 1);
            var peerActions = new FlowLayoutPanel { Dock = DockStyle.Fill, WrapContents = false, Margin = Padding.Empty };
            trustButton = ActionButton("Load other PC's identity", "TrustDeviceButton", false); trustButton.Width = 235;
            forgetButton = ActionButton("Remove pairing", "ForgetDeviceButton", false); forgetButton.Width = 160;
            trustButton.Click += async delegate { await ImportIdentity(); }; forgetButton.Click += async delegate { await ForgetIdentity(); };
            peerActions.Controls.Add(trustButton); peerActions.Controls.Add(forgetButton); peerGrid.Controls.Add(peerActions, 0, 2);
            peer.Controls.Add(peerGrid); stack.Controls.Add(peer, 0, 4);

            var session = new Card { Dock = DockStyle.Fill }; var sessionGrid = new TableLayoutPanel { Dock = DockStyle.Fill, ColumnCount = 1, RowCount = 3 };
            sessionGrid.RowStyles.Add(new RowStyle(SizeType.Absolute, 30)); sessionGrid.RowStyles.Add(new RowStyle(SizeType.Absolute, 42)); sessionGrid.RowStyles.Add(new RowStyle(SizeType.Percent, 100));
            sessionGrid.Controls.Add(Label("3   Test the connection", 13, FontStyle.Bold), 0, 0);
            var endpointRow = new FlowLayoutPanel { Dock = DockStyle.Fill, WrapContents = false, Margin = Padding.Empty };
            address = new ComboBox { Width = 230, Name = "ConnectionAddress", DropDownStyle = ComboBoxStyle.DropDown };
            address.Items.AddRange(Files.Addresses().Cast<object>().ToArray());
            endpointRow.Controls.Add(address);
            endpointRow.Controls.Add(new Label { Text = "Port", AutoSize = false, Width = 40, Height = 28, TextAlign = ContentAlignment.MiddleCenter });
            port = new NumericUpDown { Minimum = 1, Maximum = 65535, Value = preferences.Port, Width = 88, Name = "ConnectionPort" };
            endpointRow.Controls.Add(port); sessionGrid.Controls.Add(endpointRow, 0, 1);
            connectionButton = ActionButton("Start host test", "StartConnectionButton", true); connectionButton.Width = 230;
            connectionButton.Click += async delegate { await StartConnection(); }; sessionGrid.Controls.Add(connectionButton, 0, 2);
            session.Controls.Add(sessionGrid); stack.Controls.Add(session, 0, 5);
            connectionResult = Label("", 10, FontStyle.Regular); connectionResult.Name = "ConnectionStatus"; stack.Controls.Add(connectionResult, 0, 6);
            LoadAddress(); RefreshPeer();
        }

        private void LoadAddress()
        {
            loading = true; address.Text = preferences.Role == "host" ? preferences.HostAddress : preferences.ClientAddress;
            if (preferences.Role == "host" && address.Text == "127.0.0.1" && address.Items.Count > 1) address.SelectedIndex = 0;
            connectionButton.Text = preferences.Role == "host" ? "Start host test" : "Connect & run test";
            connectionResult.ForeColor = Palette.Muted;
            connectionResult.Text = preferences.Role == "host"
                ? "Select this PC's LAN or tunnel address. Start here, then start the client within 30 seconds."
                : "Enter the address shown on the host. Both PCs must verify each other's public identity first.";
            loading = false;
        }
        private void SaveAddress()
        {
            if (address == null) return;
            if (preferences.Role == "host") preferences.HostAddress = address.Text; else preferences.ClientAddress = address.Text;
            preferences.Port = (int)port.Value;
        }
        private void RefreshPeer()
        {
            peerLabel.Text = preferences.Peer == "" ? "Copy the other PC's public identity file here, then load it and compare fingerprints."
                : "Verified peer: " + preferences.Peer.Substring(0, 16) + "...  (loaded identities are checked again when connecting)";
        }
        private void UpdateEnabled()
        {
            foreach (Button action in actions) action.Enabled = !busy;
            cancelButton.Enabled = busy; cancelButton.Visible = busy; exportButton.Enabled = !busy && lastReport != null;
            progress.Visible = busy; role.Enabled = address.Enabled = port.Enabled = !busy;
            bool ready = ownFingerprint != "" && !busy;
            saveIdentityButton.Enabled = trustButton.Enabled = ready;
            forgetButton.Enabled = ready && preferences.Peer != "";
            connectionButton.Enabled = ready && preferences.Peer != "";
        }
        private void SetFooter(string text) { footer.Text = text; }
        private void KeepReport(string name, Dictionary<string, object> report)
        {
            lastReport = new Dictionary<string, object> { { "ui_report_schema", 1 }, { "check", name },
                { "recorded_at_utc", DateTime.UtcNow.ToString("o", CultureInfo.InvariantCulture) }, { "report", report } };
            string text = Json.Serializer().Serialize(lastReport);
            details.Text = text.Length > 100000 ? text.Substring(0, 100000) + "\r\n[Display shortened; export for complete report.]" : text;
        }
        private async Task Work(string title, Func<CancellationToken, Task> operation)
        {
            if (busy) return;
            busy = true; cancellation = new CancellationTokenSource();
            // Move focus before disabling the initiating button. Otherwise WinForms
            // can select the details box and scroll the page away from its heading.
            navigation[currentPage].Focus();
            UpdateEnabled(); SetFooter(title);
            try { await operation(cancellation.Token); }
            catch (OperationCanceledException) {
                if (runningCard != null) runningCard.Set("CANCELLED", "Check cancelled. Run it again when ready.");
                connectionResult.Text = "Check cancelled. Start a fresh test when ready."; SetFooter("Check cancelled. No active connection.");
            }
            catch (Exception error) {
                string message = error.Message;
                if (runningCard != null) runningCard.Set("FAILED", "The check could not complete. See details below.");
                KeepReport(title, new Dictionary<string, object> { { "status", "FAILED" }, { "diagnostic", message } });
                bool timeout = message.IndexOf("timed out", StringComparison.OrdinalIgnoreCase) >= 0 || message.IndexOf("elapsed", StringComparison.OrdinalIgnoreCase) >= 0;
                string friendly = timeout ? "Connection timed out. Check the address, pairing on both PCs, and private-network firewall access."
                    : message.Length > 230 ? message.Substring(0, 230) + "... See Checks / details." : message;
                connectionResult.Text = friendly; connectionResult.ForeColor = Palette.Red; SetFooter(friendly);
            }
            finally {
                runningCard = null; busy = false; cancellation.Dispose(); cancellation = null; UpdateEnabled();
                if (closeWhenFinished) Close();
            }
        }
        private async Task Check(bool network, CancellationToken token)
        {
            runningCard = network ? networkCard : recoveryCard;
            runningCard.Set("RUNNING", network ? "Checking a secure local connection..." : "Checking recovery scenarios...");
            string name = network ? "Local connection" : "Input recovery";
            SetFooter(name + " check running...");
            ToolResult result = await Runner.Run(Files.Tool("bricks-host.exe"),
                new[] { network ? "probe-selftest" : "recovery-selftest" }, token, 20000, false);
            result.RequireSuccess(); var report = Json.Parse(result.Output);
            CheckResult checkedResult = network ? Reports.Probe(report, true) : Reports.Recovery(report);
            runningCard.Set(checkedResult.State, checkedResult.Summary); KeepReport(name, report); runningCard = null;
        }
        private async Task RunAutomatic()
        {
            await Work("Running automatic checks...", async token => {
                await Check(false, token); await Check(true, token);
                SetFooter("Both automatic checks passed. No active connection.");
            });
        }
        private async Task RunSingle(bool network)
        {
            await Work("Running check...", async token => { await Check(network, token); SetFooter("Check passed. No active connection."); });
        }
        private async Task RunInput()
        {
            await Work("Input window open. Click Run test, then close it for results.", async token => {
                runningCard = inputCard; inputCard.Set("OPEN", "Click Run test in the separate input window, then close it.");
                ToolResult result = await Runner.Run(Files.Tool("fixture.exe"), new string[0], token, 0, true);
                result.RequireSuccess(); var report = Json.Parse(result.Output); CheckResult checkedResult = Reports.Input(report);
                inputCard.Set(checkedResult.State, checkedResult.Summary); KeepReport("Keyboard and mouse", report);
                SetFooter(checkedResult.State == "PASS" ? "Input test passed. No active connection." : checkedResult.Summary);
            });
        }

        private string RoleTool { get { return "bricks-" + preferences.Role + ".exe"; } }
        private string IdentityReference { get { return "os-store:bricks-" + preferences.Role; } }
        private async Task<ToolResult> Identity(CancellationToken token, params string[] arguments)
        {
            var result = await Runner.Run(Files.Tool(RoleTool), arguments, token, 10000, false); result.RequireSuccess(); return result;
        }
        private async Task PrepareDevice()
        {
            await Work("Setting up this device...", async token => {
                string path = Path.Combine(Files.LocalAppData, "Bricks", "identities", "bricks-" + preferences.Role, "identity.json");
                ToolResult result = await Identity(token, "identity", File.Exists(path) ? "show" : "init", IdentityReference);
                Match match = Regex.Match(result.Output.Trim(), "\\Aidentity_sha256=([0-9a-f]{64})\\z");
                if (!match.Success) throw new InvalidDataException("The device fingerprint could not be read.");
                ownFingerprint = match.Groups[1].Value; fingerprint.Text = Files.FormatFingerprint(ownFingerprint);
                SaveAddress(); Files.SavePreferences(preferences); SetFooter("Device ready. Save its public identity to share with your other PC.");
            });
        }
        private async Task ExportIdentity()
        {
            using (var dialog = new SaveFileDialog { Filter = "Public identity (*.der)|*.der", FileName = "Bricks-" + preferences.Role + "-public.der", Title = "Save this PC's public identity", OverwritePrompt = true })
            {
                if (dialog.ShowDialog(this) != DialogResult.OK) return;
                string destination = dialog.FileName;
                await Work("Saving public identity...", async token => {
                    using (var folder = new WorkFolder()) {
                        string file = Path.Combine(folder.PathName, "public.der");
                        await Identity(token, "identity", "export", IdentityReference, file);
                        File.Copy(file, destination, true);
                    }
                    SetFooter("Public identity saved. The private key stays protected on this PC.");
                });
            }
        }
        private async Task ImportIdentity()
        {
            byte[] certificate;
            using (var dialog = new OpenFileDialog { Filter = "Public identity (*.der)|*.der", Title = "Choose the other PC's public identity" })
            {
                if (dialog.ShowDialog(this) != DialogResult.OK) return;
                try { certificate = Files.Certificate(dialog.FileName); }
                catch (Exception error) { MessageBox.Show(this, error.Message, "Could not load public identity", MessageBoxButtons.OK, MessageBoxIcon.Error); return; }
            }
            string expected = Files.Fingerprint(certificate);
            string text = "Compare ALL FOUR lines with the fingerprint shown under Your device on the OTHER PC.\r\n\r\n"
                + Files.FormatFingerprint(expected) + "\r\n\r\nApprove only if they match through a trusted comparison (for example, looking at the other screen).\r\n\r\n"
                + (preferences.Role == "host" ? "This permits the device to request control of this host. This preview only runs synthetic connection tests.\r\n\r\n" : "")
                + "Do all four lines match?";
            if (MessageBox.Show(this, text, "Verify the other PC", MessageBoxButtons.YesNo, MessageBoxIcon.Question, MessageBoxDefaultButton.Button2) != DialogResult.Yes) return;
            await Work("Saving verified pairing...", async token => {
                using (var folder = new WorkFolder()) {
                    string snapshot = Path.Combine(folder.PathName, "peer.der"); File.WriteAllBytes(snapshot, certificate);
                    // Reapproving this exact fingerprint invalidates its previous
                    // grant before an explicit replacement; never silently trust
                    // a changed identity or reuse a possibly revoked old lease.
                    await Identity(token, "identity", "revoke", IdentityReference, expected);
                    var arguments = new List<string> { "identity", "trust", IdentityReference, snapshot, expected };
                    if (preferences.Role == "host") arguments.Add("--allow-control");
                    await Identity(token, arguments.ToArray());
                }
                preferences.Peer = expected; SaveAddress(); Files.SavePreferences(preferences); RefreshPeer();
                SetFooter("Verified pairing saved. Repeat verification on the other PC before testing.");
            });
        }
        private async Task ForgetIdentity()
        {
            await Work("Removing pairing...", async token => {
                await Identity(token, "identity", "revoke", IdentityReference, preferences.Peer);
                preferences.Peer = ""; Files.SavePreferences(preferences); RefreshPeer(); SetFooter("Pairing removed. No active connection.");
            });
        }
        private async Task StartConnection()
        {
            await Work("Starting connection test...", async token => {
                SaveAddress(); string selected = preferences.Role == "host" ? preferences.HostAddress : preferences.ClientAddress;
                string config = Files.Configuration(preferences.Role, selected, preferences.Port);
                Files.SavePreferences(preferences); connectionResult.ForeColor = Palette.Muted;
                connectionResult.Text = preferences.Role == "host" ? "Waiting for the verified client. Start its test within 30 seconds..." : "Connecting to the verified host...";
                using (var folder = new WorkFolder()) {
                    string path = Path.Combine(folder.PathName, "session.toml"); File.WriteAllText(path, config, new UTF8Encoding(false));
                    string[] arguments = preferences.Role == "host"
                        ? new[] { "probe-host", path, preferences.Peer, "--approve-control" }
                        : new[] { "probe-client", path, preferences.Peer, "100" };
                    ToolResult result = await Runner.Run(Files.Tool(RoleTool), arguments, token, 100000, false); result.RequireSuccess();
                    var report = Json.Parse(result.Output);
                    CheckResult checkedResult = preferences.Role == "host" ? Reports.Host(report) : Reports.Probe(report, false);
                    connectionResult.Text = checkedResult.Summary; connectionResult.ForeColor = Palette.Teal;
                    KeepReport("Connection test (" + preferences.Role + ")", report); SetFooter("Connection test passed and ended. No active connection.");
                }
            });
        }
        private void ExportReport()
        {
            if (lastReport == null) return;
            using (var dialog = new SaveFileDialog { Filter = "JSON report (*.json)|*.json", FileName = "Bricks-report.json", OverwritePrompt = true })
            {
                if (dialog.ShowDialog(this) != DialogResult.OK) return;
                try { File.WriteAllText(dialog.FileName, Json.Serializer().Serialize(lastReport), new UTF8Encoding(false)); SetFooter("Report exported."); }
                catch (Exception error) { MessageBox.Show(this, error.Message, "Report could not be saved", MessageBoxButtons.OK, MessageBoxIcon.Error); }
            }
        }
    }

    internal static class Program
    {
        [STAThread]
        private static void Main()
        {
            Application.EnableVisualStyles(); Application.SetCompatibleTextRenderingDefault(false);
            Application.SetUnhandledExceptionMode(UnhandledExceptionMode.CatchException);
            Application.ThreadException += delegate(object sender, ThreadExceptionEventArgs e) {
                MessageBox.Show(e.Exception.Message, "Bricks could not finish this action", MessageBoxButtons.OK, MessageBoxIcon.Error);
            };
            try { Application.Run(new BricksForm()); }
            catch (Exception error) { MessageBox.Show(error.Message, "Bricks could not start", MessageBoxButtons.OK, MessageBoxIcon.Error); }
        }
    }
}
