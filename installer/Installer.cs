// Foldex-Installer (Windows). build.bat kompiliert diese Datei mit dem C#-Compiler von Windows (.NET Framework 4,
// Sprachstand C# 5) und hängt foldex.exe als Nutzdaten an. Installiert für den angemeldeten Benutzer – ohne
// Administratorrechte – nach %LOCALAPPDATA%\Programs\Foldex (änderbar).
//
// Aufruf:
//   Foldex-Installer.exe                    Assistent (Installieren bzw. Aktualisieren)
//   Foldex-Installer.exe /silent [/dir=…] [/ai=id,id] [/nodesktop] [/launch]
//   /noupdate: nicht auf GitHub nach einer neueren Version sehen (sonst lädt der Assistent sie bei Bedarf)
//   Foldex-Deinstallation.exe /uninstall [/silent] [/purge]   (/purge: auch Einstellungen und KI-Modelle)

using System;
using System.Collections;
using System.Collections.Generic;
using System.Diagnostics;
using System.Drawing;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Web.Script.Serialization;
using System.Windows.Forms;
using Microsoft.Win32;

static class Program
{
    [DllImport("user32.dll")]
    static extern bool SetProcessDPIAware();

    [STAThread]
    static int Main(string[] args)
    {
        try { SetProcessDPIAware(); } catch (Exception) { }
        Application.EnableVisualStyles();
        Application.SetCompatibleTextRenderingDefault(false);
        Opts o = Opts.Parse(args);
        Setup.Sandbox = o.Sandbox;
        try
        {
            if (o.UninstallRun != null) { return Setup.UninstallRun(o); }
            if (o.Uninstall) { return Setup.UninstallStart(o); }
            if (o.Silent) { return Setup.SilentInstall(o); }
            // gibt es auf GitHub schon eine neuere Version? Dann diesen Installer laden und starten
            if (!o.NoUpdate && o.Sandbox == null && Online.Offer()) { return 0; }
            Application.Run(new Wizard(o));
            return 0;
        }
        catch (Exception ex)
        {
            if (!o.Silent) { MessageBox.Show("Fehler: " + ex.Message, "Foldex", MessageBoxButtons.OK, MessageBoxIcon.Error); }
            return 1;
        }
    }
}

/// Programmsymbol in hoher Auflösung (eingebettet von build.bat: /resource:app.ico)
static class Art
{
    static Icon big;

    public static Icon AppIcon()
    {
        if (big != null) { return big; }
        try
        {
            using (Stream s = typeof(Art).Assembly.GetManifestResourceStream("app.ico"))
            {
                if (s != null) { big = new Icon(s, 256, 256); }
            }
        }
        catch (Exception) { }
        if (big == null) { big = Icon.ExtractAssociatedIcon(Application.ExecutablePath); }
        return big;
    }

    /// größtes Bild aus der .ico direkt lesen (die 256-px-Stufe ist als PNG gespeichert; Icon.ToBitmap
    /// liefert sonst oft nur eine kleine Stufe)
    static Bitmap Largest()
    {
        try
        {
            using (Stream s = typeof(Art).Assembly.GetManifestResourceStream("app.ico"))
            {
                if (s == null) { return null; }
                byte[] d = new byte[s.Length];
                s.Read(d, 0, d.Length);
                int n = BitConverter.ToUInt16(d, 4);
                int best = -1, bestW = 0;
                for (int i = 0; i < n; i++)
                {
                    int w = d[6 + 16 * i] == 0 ? 256 : d[6 + 16 * i];
                    if (w > bestW) { bestW = w; best = i; }
                }
                if (best < 0) { return null; }
                int size = BitConverter.ToInt32(d, 6 + 16 * best + 8);
                int off = BitConverter.ToInt32(d, 6 + 16 * best + 12);
                // PNG-Kennung
                if (d[off] == 0x89 && d[off + 1] == 0x50)
                {
                    return new Bitmap(new MemoryStream(d, off, size));
                }
            }
        }
        catch (Exception) { }
        return null;
    }

    /// Logo in genau dieser Größe, hochwertig verkleinert
    public static Bitmap Logo(int px)
    {
        Bitmap src = Largest() ?? AppIcon().ToBitmap();
        Bitmap b = new Bitmap(px, px);
        using (Graphics g = Graphics.FromImage(b))
        {
            g.InterpolationMode = System.Drawing.Drawing2D.InterpolationMode.HighQualityBicubic;
            g.SmoothingMode = System.Drawing.Drawing2D.SmoothingMode.HighQuality;
            g.PixelOffsetMode = System.Drawing.Drawing2D.PixelOffsetMode.HighQuality;
            g.DrawImage(src, new Rectangle(0, 0, px, px));
        }
        src.Dispose();
        return b;
    }
}

/// neueste Version aus dem GitHub-Repository
static class Online
{
    public const string VersionUrl = "https://raw.githubusercontent.com/xJetouss/Foldex/main/Cargo.toml";
    public const string InstallerUrl = "https://github.com/xJetouss/Foldex/raw/main/dist/Foldex-Installer.exe";

    static int[] Parse(string v)
    {
        int[] r = new int[3];
        string[] p = v.Trim().TrimStart('v').Split('.');
        for (int i = 0; i < 3 && i < p.Length; i++)
        {
            string d = "";
            foreach (char c in p[i]) { if (char.IsDigit(c)) { d += c; } else { break; } }
            int.TryParse(d, out r[i]);
        }
        return r;
    }

    public static bool Newer(string a, string than)
    {
        int[] x = Parse(a), y = Parse(than);
        for (int i = 0; i < 3; i++) { if (x[i] != y[i]) { return x[i] > y[i]; } }
        return false;
    }

    class Client : System.Net.WebClient
    {
        public int Timeout = 8000;
        protected override System.Net.WebRequest GetWebRequest(Uri u)
        {
            System.Net.WebRequest r = base.GetWebRequest(u);
            r.Timeout = Timeout;
            return r;
        }
    }

    /// Version auf GitHub (null ohne Verbindung)
    public static string Latest()
    {
        try
        {
            System.Net.ServicePointManager.SecurityProtocol |= (System.Net.SecurityProtocolType)3072; // TLS 1.2
            using (Client c = new Client())
            {
                c.Headers.Add("User-Agent", "Foldex-Installer");
                foreach (string l0 in c.DownloadString(VersionUrl).Split('\n'))
                {
                    string l = l0.Trim();
                    if (l.StartsWith("version"))
                    {
                        string v = l.Substring(7).TrimStart(' ', '=').Trim().Trim('"');
                        if (v.Length > 0) { return v; }
                    }
                }
            }
        }
        catch (Exception) { }
        return null;
    }

    /// neuere Version anbieten; true = neuer Installer läuft (dieser endet)
    public static bool Offer()
    {
        string v = Latest();
        if (v == null || !Newer(v, Ver.Value)) { return false; }
        DialogResult a = MessageBox.Show("Auf GitHub gibt es schon Foldex " + v + " (dieser Installer enthält " + Ver.Value + ").\r\n\r\nDie neueste Version jetzt herunterladen und installieren?", "Foldex – neue Version", MessageBoxButtons.YesNo, MessageBoxIcon.Information);
        if (a != DialogResult.Yes) { return false; }
        string dest = Path.Combine(Path.GetTempPath(), "Foldex-Installer-" + v + ".exe");
        try
        {
            using (Client c = new Client())
            {
                c.Timeout = 300000;
                c.Headers.Add("User-Agent", "Foldex-Installer");
                Cursor.Current = Cursors.WaitCursor;
                c.DownloadFile(InstallerUrl, dest);
            }
            if (new FileInfo(dest).Length < (1 << 20)) { throw new Exception("Datei unvollständig"); }
            Process.Start(dest, "/noupdate");
            return true;
        }
        catch (Exception ex)
        {
            MessageBox.Show("Download fehlgeschlagen (" + ex.Message + "). Es wird Version " + Ver.Value + " installiert.", "Foldex", MessageBoxButtons.OK, MessageBoxIcon.Warning);
            return false;
        }
    }
}

/// Befehlszeile
class Opts
{
    public bool Silent, Uninstall, Purge, NoDesktop, Launch, NoUpdate;
    public string Dir, Ai, UninstallRun;
    /// nur zum Testen: Verknüpfungen und Registry-Einträge in einen Testbereich statt ins echte System
    public string Sandbox;

    public static Opts Parse(string[] args)
    {
        Opts o = new Opts();
        foreach (string a0 in args)
        {
            string a = a0.Trim();
            string l = a.ToLowerInvariant();
            if (l == "/silent") { o.Silent = true; }
            else if (l == "/uninstall") { o.Uninstall = true; }
            else if (l == "/purge") { o.Purge = true; }
            else if (l == "/nodesktop") { o.NoDesktop = true; }
            else if (l == "/launch") { o.Launch = true; }
            else if (l == "/noupdate") { o.NoUpdate = true; }
            else if (l.StartsWith("/dir=")) { o.Dir = a.Substring(5).Trim('"'); }
            else if (l.StartsWith("/ai=")) { o.Ai = a.Substring(4).Trim('"'); }
            else if (l.StartsWith("/uninstall-run=")) { o.UninstallRun = a.Substring(15).Trim('"'); }
            else if (l.StartsWith("/sandbox=")) { o.Sandbox = a.Substring(9).Trim('"'); }
        }
        return o;
    }
}

/// eine wählbare KI-Komponente (aus „foldex.exe --list-ai“)
class AiComp
{
    public string Id, Group, Name, Desc;
    public long Mb;
    public bool Installed, Recommended;
}

/// Installieren, Aktualisieren, Deinstallieren
static class Setup
{
    public const string App = "Foldex";
    public const string Exe = "foldex.exe";
    public const string Uninstaller = "Foldex-Deinstallation.exe";
    const string Marker = "FOLDEX-PAYLOAD-BEGIN";
    public static string Sandbox;

    /// Registry-Pfad (im Testmodus unter Software\FoldexInstallerTest)
    static string Reg(string path) { return Sandbox == null ? path : @"Software\FoldexInstallerTest\" + path; }
    static string UninstKey { get { return Reg(@"Software\Microsoft\Windows\CurrentVersion\Uninstall\Foldex"); } }
    static string[] ContextKeys { get { return new string[] { Reg(@"Software\Classes\Directory\shell\Foldex"), Reg(@"Software\Classes\Directory\Background\shell\Foldex"), Reg(@"Software\Classes\Drive\shell\Foldex") }; } }

    public static string DefaultDir()
    {
        string old = InstalledDir();
        if (old != null) { return old; }
        return Path.Combine(Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "Programs"), App);
    }

    /// bereits installiert? (Ordner aus dem Eintrag unter „Apps“)
    public static string InstalledDir()
    {
        using (RegistryKey k = Registry.CurrentUser.OpenSubKey(UninstKey))
        {
            if (k == null) { return null; }
            string d = k.GetValue("InstallLocation") as string;
            return (d != null && File.Exists(Path.Combine(d, Exe))) ? d : null;
        }
    }

    public static string InstalledVersion()
    {
        using (RegistryKey k = Registry.CurrentUser.OpenSubKey(UninstKey))
        {
            return k == null ? null : k.GetValue("DisplayVersion") as string;
        }
    }

    /// angehängte foldex.exe aus dieser Datei lesen
    public static byte[] Payload()
    {
        byte[] all = File.ReadAllBytes(Application.ExecutablePath);
        byte[] m = Encoding.ASCII.GetBytes(Marker);
        for (int i = all.Length - m.Length - 16; i >= 0; i--)
        {
            bool hit = true;
            for (int k = 0; k < m.Length; k++) { if (all[i + k] != m[k]) { hit = false; break; } }
            if (!hit) { continue; }
            long len = Convert.ToInt64(Encoding.ASCII.GetString(all, i + m.Length, 16), 16);
            byte[] data = new byte[len];
            Array.Copy(all, i + m.Length + 16, data, 0, len);
            return data;
        }
        throw new Exception("Installationsdaten fehlen – bitte den Installer neu erstellen (build.bat).");
    }

    static string tempExe;

    /// foldex.exe vorab in einen temporären Ordner (für die Liste der KI-Komponenten)
    public static string TempExe()
    {
        if (tempExe != null && File.Exists(tempExe)) { return tempExe; }
        string d = Path.Combine(Path.GetTempPath(), "foldex-setup-" + Process.GetCurrentProcess().Id);
        Directory.CreateDirectory(d);
        tempExe = Path.Combine(d, Exe);
        File.WriteAllBytes(tempExe, Payload());
        return tempExe;
    }

    public static void CleanTemp()
    {
        try { if (tempExe != null) { Directory.Delete(Path.GetDirectoryName(tempExe), true); } } catch (Exception) { }
    }

    /// KI-Komponenten von Foldex selbst erfragen (Name, Größe, schon installiert)
    public static List<AiComp> Catalog()
    {
        ProcessStartInfo si = new ProcessStartInfo(TempExe(), "--list-ai");
        si.UseShellExecute = false;
        si.RedirectStandardOutput = true;
        si.CreateNoWindow = true;
        si.StandardOutputEncoding = Encoding.UTF8;
        string json;
        using (Process p = Process.Start(si))
        {
            json = p.StandardOutput.ReadToEnd();
            p.WaitForExit(20000);
        }
        List<AiComp> list = new List<AiComp>();
        object[] arr = new JavaScriptSerializer().DeserializeObject(json.Trim()) as object[];
        if (arr == null) { return list; }
        foreach (object x in arr)
        {
            Dictionary<string, object> d = x as Dictionary<string, object>;
            if (d == null) { continue; }
            AiComp c = new AiComp();
            c.Id = (string)d["id"];
            c.Group = (string)d["group"];
            c.Name = (string)d["name"];
            c.Desc = (string)d["desc"];
            c.Mb = Convert.ToInt64(d["mb"]);
            c.Installed = (bool)d["installed"];
            c.Recommended = (bool)d["recommended"];
            list.Add(c);
        }
        return list;
    }

    /// laufendes Foldex beenden (sonst ist foldex.exe gesperrt)
    public static void KillRunning(string dir)
    {
        foreach (Process p in Process.GetProcessesByName("foldex"))
        {
            try
            {
                // nur Foldex aus dem Installationsordner (andere Kopien, z. B. dist\foldex.exe, laufen weiter)
                string f = p.MainModule.FileName;
                if (dir == null || f.StartsWith(dir.TrimEnd('\\') + "\\", StringComparison.OrdinalIgnoreCase))
                {
                    p.Kill();
                    p.WaitForExit(5000);
                }
            }
            catch (Exception) { }
        }
    }

    public static long FreeBytes(string dir)
    {
        try
        {
            string root = Path.GetPathRoot(Path.GetFullPath(dir));
            return new DriveInfo(root).AvailableFreeSpace;
        }
        catch (Exception) { return -1; }
    }

    static void Shortcut(string link, string target, string desc)
    {
        Directory.CreateDirectory(Path.GetDirectoryName(link));
        Type t = Type.GetTypeFromProgID("WScript.Shell");
        object shell = Activator.CreateInstance(t);
        object l = t.InvokeMember("CreateShortcut", System.Reflection.BindingFlags.InvokeMethod, null, shell, new object[] { link });
        Type lt = l.GetType();
        lt.InvokeMember("TargetPath", System.Reflection.BindingFlags.SetProperty, null, l, new object[] { target });
        lt.InvokeMember("WorkingDirectory", System.Reflection.BindingFlags.SetProperty, null, l, new object[] { Path.GetDirectoryName(target) });
        lt.InvokeMember("IconLocation", System.Reflection.BindingFlags.SetProperty, null, l, new object[] { target + ",0" });
        lt.InvokeMember("Description", System.Reflection.BindingFlags.SetProperty, null, l, new object[] { desc });
        lt.InvokeMember("Save", System.Reflection.BindingFlags.InvokeMethod, null, l, null);
        Marshal.ReleaseComObject(l);
        Marshal.ReleaseComObject(shell);
    }

    static string StartMenuLink() { return Path.Combine(Sandbox != null ? Path.Combine(Sandbox, "Startmenue") : Environment.GetFolderPath(Environment.SpecialFolder.Programs), App + ".lnk"); }
    static string DesktopLink() { return Path.Combine(Sandbox != null ? Path.Combine(Sandbox, "Desktop") : Environment.GetFolderPath(Environment.SpecialFolder.DesktopDirectory), App + ".lnk"); }

    /// Schritte der Installation; `report(Anteil 0..1, Text)`
    public static void Install(string dir, bool desktop, bool startMenu, string version, Action<double, string> report)
    {
        report(0.02, "Laufendes Foldex wird beendet …");
        KillRunning(dir);
        report(0.08, "Programmdateien werden kopiert …");
        Directory.CreateDirectory(dir);
        string exe = Path.Combine(dir, Exe);
        byte[] data = Payload();
        for (int i = 0; ; i++)
        {
            try { File.WriteAllBytes(exe, data); break; }
            catch (IOException) { if (i > 10) { throw; } Thread.Sleep(500); KillRunning(dir); }
        }
        // Deinstallations-Programm (dieser Installer selbst) in den Programmordner
        string un = Path.Combine(dir, Uninstaller);
        if (!string.Equals(Path.GetFullPath(Application.ExecutablePath), Path.GetFullPath(un), StringComparison.OrdinalIgnoreCase))
        {
            File.Copy(Application.ExecutablePath, un, true);
        }
        report(0.55, "Verknüpfungen werden angelegt …");
        if (startMenu) { Shortcut(StartMenuLink(), exe, "Foldex – Dateien, Cloud und KI"); }
        else { TryDelete(StartMenuLink()); }
        if (desktop) { Shortcut(DesktopLink(), exe, "Foldex – Dateien, Cloud und KI"); }
        else { TryDelete(DesktopLink()); }
        report(0.7, "Eintrag unter „Apps“ wird angelegt …");
        using (RegistryKey k = Registry.CurrentUser.CreateSubKey(UninstKey))
        {
            k.SetValue("DisplayName", App);
            k.SetValue("DisplayVersion", version);
            k.SetValue("Publisher", App);
            k.SetValue("DisplayIcon", exe + ",0");
            k.SetValue("InstallLocation", dir);
            k.SetValue("UninstallString", "\"" + un + "\" /uninstall");
            k.SetValue("QuietUninstallString", "\"" + un + "\" /uninstall /silent");
            k.SetValue("InstallDate", DateTime.Now.ToString("yyyyMMdd"));
            k.SetValue("EstimatedSize", (int)((data.Length + new FileInfo(un).Length) / 1024), RegistryValueKind.DWord);
            k.SetValue("NoModify", 1, RegistryValueKind.DWord);
            k.SetValue("NoRepair", 1, RegistryValueKind.DWord);
        }
        // „Mit Foldex öffnen“ gibt es nicht mehr: Einträge älterer Versionen entfernen
        RemoveContextMenu();
        report(0.95, "Foldex ist installiert");
    }

    /// Einträge „Mit Foldex öffnen“ älterer Versionen im Explorer-Kontextmenü entfernen
    static void RemoveContextMenu()
    {
        foreach (string k in ContextKeys) { try { Registry.CurrentUser.DeleteSubKeyTree(k); } catch (Exception) { } }
    }


    static void TryDelete(string f) { try { if (File.Exists(f)) { File.Delete(f); } } catch (Exception) { } }

    /// KI-Komponenten über Foldex laden („--install-ai“); `report(Prozent, Text)`; Rückgabe: Fehlertext oder null
    public static string InstallAi(string exe, List<string> ids, Action<double, string> report, Func<bool> cancelled)
    {
        if (ids.Count == 0) { return null; }
        ProcessStartInfo si = new ProcessStartInfo(exe, "--install-ai \"" + string.Join(",", ids.ToArray()) + "\"");
        si.UseShellExecute = false;
        si.RedirectStandardOutput = true;
        si.CreateNoWindow = true;
        si.StandardOutputEncoding = Encoding.UTF8;
        string err = null;
        using (Process p = Process.Start(si))
        {
            string line;
            while ((line = p.StandardOutput.ReadLine()) != null)
            {
                if (cancelled()) { try { p.Kill(); } catch (Exception) { } return "abgebrochen"; }
                if (line.StartsWith("PROGRESS "))
                {
                    string rest = line.Substring(9);
                    int sp = rest.IndexOf(' ');
                    double pct;
                    double.TryParse(sp > 0 ? rest.Substring(0, sp) : rest, System.Globalization.NumberStyles.Float, System.Globalization.CultureInfo.InvariantCulture, out pct);
                    report(pct, sp > 0 ? rest.Substring(sp + 1) : "");
                }
                else if (line.StartsWith("ERROR ")) { err = line.Substring(6); }
            }
            p.WaitForExit();
        }
        return err;
    }

    public static int SilentInstall(Opts o)
    {
        string dir = o.Dir ?? DefaultDir();
        Install(dir, !o.NoDesktop, true, Ver.Value, delegate (double f, string t) { });
        if (!string.IsNullOrEmpty(o.Ai))
        {
            List<string> ids = new List<string>(o.Ai.Split(','));
            string e = InstallAi(Path.Combine(dir, Exe), ids, delegate (double f, string t) { }, delegate { return false; });
            if (e != null) { return 2; }
        }
        // Update aus Foldex heraus: danach die neue Version öffnen
        if (o.Launch) { try { Process.Start(Path.Combine(dir, Exe)); } catch (Exception) { } }
        return 0;
    }

    // ── Deinstallieren ──

    /// Schritt 1: Kopie in den Temp-Ordner starten (eine laufende EXE kann ihren Ordner nicht löschen)
    public static int UninstallStart(Opts o)
    {
        string dir = InstalledDir() ?? Path.GetDirectoryName(Application.ExecutablePath);
        if (!o.Silent)
        {
            using (UninstallDialog d = new UninstallDialog())
            {
                if (d.ShowDialog() != DialogResult.OK) { return 0; }
                o.Purge = d.Purge;
            }
        }
        string tmp = Path.Combine(Path.GetTempPath(), "foldex-deinstallation-" + Guid.NewGuid().ToString("N").Substring(0, 8) + ".exe");
        File.Copy(Application.ExecutablePath, tmp, true);
        string a = "\"/uninstall-run=" + dir + "\"" + (o.Silent ? " /silent" : "") + (o.Purge ? " /purge" : "") + (Sandbox != null ? " \"/sandbox=" + Sandbox + "\"" : "");
        Process.Start(new ProcessStartInfo(tmp, a) { UseShellExecute = false });
        return 0;
    }

    /// Schritt 2 (aus dem Temp-Ordner): alles entfernen
    public static int UninstallRun(Opts o)
    {
        Thread.Sleep(800);
        string dir = o.UninstallRun;
        KillRunning(dir);
        // Cloud-Laufwerke (rclone aus Foldex) beenden – nicht im Testmodus
        foreach (Process p in Sandbox != null ? new Process[0] : Process.GetProcessesByName("rclone"))
        {
            try { if (p.MainModule.FileName.IndexOf("foldex", StringComparison.OrdinalIgnoreCase) >= 0) { p.Kill(); } } catch (Exception) { }
        }
        for (int i = 0; i < 10 && Directory.Exists(dir); i++)
        {
            try { Directory.Delete(dir, true); } catch (Exception) { Thread.Sleep(500); }
        }
        TryDelete(StartMenuLink());
        TryDelete(DesktopLink());
        RemoveContextMenu();
        try { Registry.CurrentUser.DeleteSubKeyTree(UninstKey); } catch (Exception) { }
        if (o.Purge && Sandbox == null)
        {
            string[] data = {
                Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData), "foldex"),
                Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "foldex"),
            };
            foreach (string d in data) { try { if (Directory.Exists(d)) { Directory.Delete(d, true); } } catch (Exception) { } }
        }
        if (!o.Silent)
        {
            MessageBox.Show("Foldex wurde entfernt." + (o.Purge ? "\r\nEinstellungen, KI-Modelle und Cloud-Anmeldungen wurden ebenfalls gelöscht." : "\r\nDeine Einstellungen und KI-Modelle sind noch da – bei einer neuen Installation geht es dort weiter."), App, MessageBoxButtons.OK, MessageBoxIcon.Information);
        }
        // diese Temp-Kopie nach dem Beenden löschen
        Process.Start(new ProcessStartInfo("cmd.exe", "/c ping -n 3 127.0.0.1 >nul & del /f /q \"" + Application.ExecutablePath + "\"") { CreateNoWindow = true, UseShellExecute = false });
        return 0;
    }
}

/// Rückfrage beim Deinstallieren
class UninstallDialog : Form
{
    CheckBox purge;
    public bool Purge { get { return purge.Checked; } }

    public UninstallDialog()
    {
        Text = "Foldex deinstallieren";
        Font = new Font("Segoe UI", 9.5f);
        FormBorderStyle = FormBorderStyle.FixedDialog;
        MaximizeBox = false; MinimizeBox = false;
        StartPosition = FormStartPosition.CenterScreen;
        ClientSize = new Size(500, 210);
        Icon = Art.AppIcon();
        Label l = new Label();
        l.Text = "Foldex wird von diesem PC entfernt: Programm, Verknüpfungen und der Eintrag unter „Apps“.";
        l.SetBounds(20, 20, 460, 44);
        purge = new CheckBox();
        purge.Text = "Auch Einstellungen, KI-Modelle, Cloud-Anmeldungen und den Suchindex löschen";
        purge.SetBounds(20, 74, 460, 40);
        Label hint = new Label();
        hint.Text = "Deine eigenen Dateien und deine Cloud-Daten bleiben erhalten.";
        hint.ForeColor = Color.DimGray;
        hint.SetBounds(20, 118, 460, 22);
        Button ok = new Button(); ok.Text = "Deinstallieren"; ok.DialogResult = DialogResult.OK; ok.SetBounds(274, 160, 110, 32);
        Button no = new Button(); no.Text = "Abbrechen"; no.DialogResult = DialogResult.Cancel; no.SetBounds(392, 160, 90, 32);
        AcceptButton = ok; CancelButton = no;
        Controls.AddRange(new Control[] { l, purge, hint, ok, no });
    }
}

/// Assistent: Willkommen → Ort → Optionen → KI → Zusammenfassung → Installation → Fertig
class Wizard : Form
{
    Opts opts;
    Panel header, body, footer;
    Label title, subtitle;
    Button back, next, cancel;
    int page;
    readonly string[] titles = { "Willkommen", "Zielordner", "Zusätzliche Aufgaben", "Komponenten", "Bereit zur Installation", "Installation", "Fertig" };
    readonly string[] subs = {
        "",
        "Wohin soll Foldex installiert werden?",
        "Welche zusätzlichen Aufgaben sollen ausgeführt werden?",
        "Optionale Komponenten auswählen – alles lässt sich später auch in Foldex hinzufügen",
        "Das Setup ist bereit, Foldex zu installieren",
        "Bitte warten, Foldex wird installiert",
        "",
    };

    // Seiten
    Panel[] pages = new Panel[7];
    TextBox dirBox;
    Label spaceLabel, aiTotal, summary, progLabel, doneLabel, doneTitle;
    CheckBox cDesktop, cStart, cLaunch, cLaunchDone;
    TreeView aiTree, edTree;
    Panel banner;
    ProgressBar prog;
    TextBox log;
    List<AiComp> catalog;
    bool installing, done, cancelRequested;

    static readonly Color Accent = Color.FromArgb(0, 120, 212);

    public Wizard(Opts o)
    {
        opts = o;
        Text = "Setup – Foldex " + Ver.Value;
        Font = new Font("Segoe UI", 9.5f);
        FormBorderStyle = FormBorderStyle.FixedDialog;
        MaximizeBox = false;
        StartPosition = FormStartPosition.CenterScreen;
        AutoScaleMode = AutoScaleMode.Dpi;
        ClientSize = new Size(760, 540);
        Icon = Art.AppIcon();

        header = new Panel(); header.Dock = DockStyle.Top; header.Height = 72; header.BackColor = Color.White;
        title = new Label(); title.Font = new Font("Segoe UI Semibold", 12.5f); title.SetBounds(24, 13, 620, 26);
        subtitle = new Label(); subtitle.ForeColor = Color.DimGray; subtitle.SetBounds(25, 40, 640, 22);
        int lp = (int)(48 * DeviceDpi / 96f);
        PictureBox logo = new PictureBox(); logo.SetBounds(694, 12, 48, 48); logo.SizeMode = PictureBoxSizeMode.Zoom;
        logo.Image = Art.Logo(Math.Max(48, lp));
        header.Controls.AddRange(new Control[] { title, subtitle, logo });
        Panel line1 = new Panel(); line1.Dock = DockStyle.Top; line1.Height = 1; line1.BackColor = Color.Gainsboro;

        footer = new Panel(); footer.Dock = DockStyle.Bottom; footer.Height = 60; footer.BackColor = Color.FromArgb(243, 243, 243);
        Panel line2 = new Panel(); line2.Dock = DockStyle.Bottom; line2.Height = 1; line2.BackColor = Color.Gainsboro;
        Label brand = new Label(); brand.Text = "Foldex " + Ver.Value; brand.ForeColor = Color.Gray; brand.SetBounds(24, 21, 200, 20);
        back = new Button(); back.Text = "< Zurück"; back.SetBounds(440, 14, 96, 32); back.Click += delegate { Go(page - 1); };
        next = new Button(); next.Text = "Weiter >"; next.SetBounds(542, 14, 96, 32); next.Click += delegate { Next(); };
        cancel = new Button(); cancel.Text = "Abbrechen"; cancel.SetBounds(650, 14, 96, 32); cancel.Click += delegate { Cancel(); };
        footer.Controls.AddRange(new Control[] { brand, back, next, cancel });

        body = new Panel(); body.Dock = DockStyle.Fill; body.Padding = new Padding(28, 20, 28, 12); body.BackColor = Color.FromArgb(250, 250, 250);
        BuildPages();
        // Willkommen/Fertig: farbiges Band links mit großem Logo (wie bei üblichen Setup-Programmen)
        banner = new Panel(); banner.Dock = DockStyle.Left; banner.Width = 190; banner.Visible = false;
        int bp = (int)(112 * DeviceDpi / 96f);
        Bitmap big = Art.Logo(Math.Max(112, bp));
        banner.Paint += delegate (object s, PaintEventArgs e)
        {
            Rectangle r = banner.ClientRectangle;
            using (System.Drawing.Drawing2D.LinearGradientBrush br = new System.Drawing.Drawing2D.LinearGradientBrush(r, Color.FromArgb(32, 36, 44), Color.FromArgb(52, 58, 70), 90f))
            {
                e.Graphics.FillRectangle(br, r);
            }
            e.Graphics.InterpolationMode = System.Drawing.Drawing2D.InterpolationMode.HighQualityBicubic;
            int sz = (int)(112 * e.Graphics.DpiX / 96f);
            e.Graphics.DrawImage(big, new Rectangle((r.Width - sz) / 2, (int)(48 * e.Graphics.DpiY / 96f), sz, sz));
            using (Font f = new Font("Segoe UI Semibold", 15f))
            using (StringFormat sf = new StringFormat { Alignment = StringAlignment.Center })
            {
                e.Graphics.TextRenderingHint = System.Drawing.Text.TextRenderingHint.ClearTypeGridFit;
                e.Graphics.DrawString("Foldex", f, Brushes.White, new RectangleF(0, (int)(176 * e.Graphics.DpiY / 96f), r.Width, 40), sf);
            }
            using (Font f = new Font("Segoe UI", 9f))
            using (StringFormat sf = new StringFormat { Alignment = StringAlignment.Center })
            using (SolidBrush b = new SolidBrush(Color.FromArgb(170, 178, 190)))
            {
                e.Graphics.DrawString("Version " + Ver.Value, f, b, new RectangleF(0, (int)(208 * e.Graphics.DpiY / 96f), r.Width, 24), sf);
            }
        };
        Controls.Add(body);
        Controls.Add(banner);
        Controls.Add(line1);
        Controls.Add(header);
        Controls.Add(line2);
        Controls.Add(footer);
        AcceptButton = next;
        FormClosing += delegate (object s, FormClosingEventArgs e)
        {
            if (installing && !done) { e.Cancel = true; Cancel(); return; }
            Setup.CleanTemp();
        };
        Go(0);
    }

    Panel NewPage()
    {
        Panel p = new Panel(); p.Dock = DockStyle.Fill; p.Visible = false; p.BackColor = body.BackColor;
        body.Controls.Add(p);
        return p;
    }

    static Label Text_(string t, int x, int y, int w, int h)
    {
        Label l = new Label(); l.Text = t; l.SetBounds(x, y, w, h); return l;
    }

    void BuildPages()
    {
        // 0 Willkommen
        Panel p0 = pages[0] = NewPage();
        string old = Setup.InstalledVersion();
        Label wt = Text_("Willkommen beim Setup von Foldex " + Ver.Value, 0, 4, 500, 64);
        wt.Font = new Font("Segoe UI Semibold", 15f);
        string welcome = "Dieser Assistent führt dich durch die Installation von Foldex.\r\n\r\n"
            + "Foldex wird nur für dein Benutzerkonto installiert. Administratorrechte sind dafür nicht nötig.";
        if (old != null && Setup.InstalledDir() != null)
        {
            welcome += "\r\n\r\nFoldex " + old + " ist bereits installiert und wird auf Version " + Ver.Value + " aktualisiert. Deine Einstellungen bleiben erhalten.";
        }
        else
        {
            welcome += "\r\n\r\nSchließe am besten andere Programme, bevor du fortfährst.";
        }
        p0.Controls.Add(wt);
        p0.Controls.Add(Text_(welcome, 0, 80, 500, 200));
        p0.Controls.Add(Text_("Klicke auf „Weiter“, um fortzufahren.", 0, 300, 500, 24));

        // 1 Ort
        Panel p1 = pages[1] = NewPage();
        p1.Controls.Add(Text_("Foldex wird in diesen Ordner installiert. Für einen anderen Ordner auf „Durchsuchen …“ klicken.", 0, 0, 690, 40));
        dirBox = new TextBox(); dirBox.Text = opts.Dir ?? Setup.DefaultDir(); dirBox.SetBounds(0, 52, 560, 28);
        dirBox.TextChanged += delegate { UpdateSpace(); };
        Button browse = new Button(); browse.Text = "Durchsuchen …"; browse.SetBounds(570, 50, 120, 30);
        browse.Click += delegate
        {
            using (FolderBrowserDialog f = new FolderBrowserDialog())
            {
                f.Description = "Ordner für Foldex wählen";
                f.SelectedPath = dirBox.Text;
                if (f.ShowDialog(this) == DialogResult.OK)
                {
                    string d = f.SelectedPath;
                    if (!d.TrimEnd('\\').EndsWith("Foldex", StringComparison.OrdinalIgnoreCase)) { d = Path.Combine(d, "Foldex"); }
                    dirBox.Text = d;
                }
            }
        };
        spaceLabel = Text_("", 0, 96, 690, 60); spaceLabel.ForeColor = Color.DimGray;
        p1.Controls.AddRange(new Control[] { dirBox, browse, spaceLabel });

        // 2 Optionen
        Panel p2 = pages[2] = NewPage();
        cStart = Check("Eintrag im Startmenü", true, 0);
        cDesktop = Check("Verknüpfung auf dem Desktop", !opts.NoDesktop, 34);
        cLaunch = Check("Foldex nach der Installation starten", true, 68);
        p2.Controls.AddRange(new Control[] { cStart, cDesktop, cLaunch });

        // 3 Komponenten: Reiter „Agents“ (Sprachmodelle) und „Editor“ (Bild, Audio … für die Editoren)
        Panel p3 = pages[3] = NewPage();
        TabControl tabs = new TabControl(); tabs.SetBounds(0, 0, 704, 268);
        TabPage tAgents = new TabPage("Agents"), tEditor = new TabPage("Editor");
        aiTree = NewTree(); edTree = NewTree();
        tAgents.Controls.Add(aiTree); tEditor.Controls.Add(edTree);
        tabs.TabPages.Add(tAgents); tabs.TabPages.Add(tEditor);
        Button rec = new Button(); rec.Text = "Empfohlene"; rec.SetBounds(0, 276, 110, 30); rec.Click += delegate { SetAi(true); };
        Button none = new Button(); none.Text = "Keine"; none.SetBounds(116, 276, 80, 30); none.Click += delegate { SetAi(false); };
        aiTotal = Text_("", 210, 282, 494, 22); aiTotal.TextAlign = ContentAlignment.MiddleRight;
        Label aiHint = Text_("Agents: Sprachmodelle für Chat und Assistent. Editor: Modelle für Bild- und Audio-Bearbeitung. Alles läuft lokal; bereits vorhandene Teile sind ausgegraut.", 0, 314, 704, 44);
        aiHint.ForeColor = Color.DimGray;
        p3.Controls.AddRange(new Control[] { tabs, rec, none, aiTotal, aiHint });

        // 4 Zusammenfassung
        Panel p4 = pages[4] = NewPage();
        summary = Text_("", 0, 0, 704, 380);
        p4.Controls.Add(summary);

        // 5 Installation
        Panel p5 = pages[5] = NewPage();
        progLabel = Text_("", 0, 0, 704, 24);
        prog = new ProgressBar(); prog.SetBounds(0, 30, 704, 22); prog.Maximum = 1000;
        log = new TextBox(); log.Multiline = true; log.ReadOnly = true; log.ScrollBars = ScrollBars.Vertical; log.SetBounds(0, 66, 704, 300); log.BackColor = Color.White;
        p5.Controls.AddRange(new Control[] { progLabel, prog, log });

        // 6 Fertig
        Panel p6 = pages[6] = NewPage();
        doneTitle = Text_("", 0, 4, 500, 40);
        doneTitle.Font = new Font("Segoe UI Semibold", 15f);
        doneLabel = Text_("", 0, 64, 500, 180);
        cLaunchDone = Check("Foldex jetzt starten", true, 250);
        p6.Controls.AddRange(new Control[] { doneTitle, doneLabel, cLaunchDone });
    }

    TreeView NewTree()
    {
        TreeView t = new TreeView();
        t.Dock = DockStyle.Fill; t.CheckBoxes = true; t.ShowLines = false; t.FullRowSelect = true; t.ItemHeight = 24; t.BorderStyle = BorderStyle.None;
        t.AfterCheck += AiChecked;
        return t;
    }

    /// welche Liste: Sprachmodelle → „Agents“, alles für die Editoren → „Editor“
    TreeView TreeFor(AiComp a) { return a.Group.StartsWith("KI-Chat") ? aiTree : edTree; }

    IEnumerable<TreeView> Trees() { yield return aiTree; yield return edTree; }

    static CheckBox Check(string t, bool on, int y)
    {
        CheckBox c = new CheckBox(); c.Text = t; c.Checked = on; c.SetBounds(0, y, 700, 28); return c;
    }

    void UpdateSpace()
    {
        long free = Setup.FreeBytes(dirBox.Text);
        spaceLabel.Text = "Benötigt: etwa 50 MB" + (free >= 0 ? "     Frei auf dem Laufwerk: " + Gb(free) : "") + "\r\nKI-Modelle werden im Benutzerordner gespeichert (%APPDATA%\\foldex) – ihre Größe steht auf der Seite „KI-Komponenten“.";
    }

    static string Gb(long bytes)
    {
        if (bytes >= 1L << 30) { return (bytes / (double)(1L << 30)).ToString("0.0") + " GB"; }
        return (bytes / (double)(1L << 20)).ToString("0") + " MB";
    }

    static string Mb(long mb)
    {
        return mb >= 1024 ? (mb / 1024.0).ToString("0.0") + " GB" : mb + " MB";
    }

    // ── KI-Liste ──

    bool loadingAi, fillingTree;

    void LoadAi()
    {
        if (catalog != null || loadingAi) { return; }
        loadingAi = true;
        aiTree.Nodes.Clear();
        aiTree.Nodes.Add("Liste wird geladen …");
        edTree.Nodes.Clear();
        edTree.Nodes.Add("Liste wird geladen …");
        Thread t = new Thread(delegate ()
        {
            List<AiComp> c;
            string err = null;
            try { c = Setup.Catalog(); } catch (Exception ex) { c = new List<AiComp>(); err = ex.Message; }
            BeginInvoke((Action)delegate { FillTree(c, err); });
        });
        t.IsBackground = true;
        t.Start();
    }

    void FillTree(List<AiComp> c, string err)
    {
        catalog = c;
        loadingAi = false;
        fillingTree = true;
        foreach (TreeView t in Trees()) { t.BeginUpdate(); t.Nodes.Clear(); }
        if (err != null) { aiTree.Nodes.Add("Liste nicht verfügbar: " + err); }
        TreeNode group = null;
        HashSet<string> pre = new HashSet<string>((opts.Ai ?? "").Split(','));
        foreach (AiComp a in c)
        {
            if (group == null || (string)group.Tag != a.Group || group.TreeView != TreeFor(a))
            {
                group = TreeFor(a).Nodes.Add(a.Group);
                group.Tag = a.Group;
                group.NodeFont = new Font(Font, FontStyle.Bold);
                group.Text = a.Group; // Breite für fette Schrift
            }
            if (group.Text == "KI-Chat (Textmodelle)") { group.Text = "Sprachmodelle"; }
            string label = a.Name + "  –  " + Mb(a.Mb) + (a.Recommended ? "  (empfohlen)" : "") + (a.Installed ? "  · installiert" : "") + "     " + a.Desc;
            TreeNode n = group.Nodes.Add(label);
            n.Tag = a;
            if (a.Installed) { n.Checked = true; n.ForeColor = Color.Gray; }
            else if (pre.Contains(a.Id)) { n.Checked = true; }
        }
        foreach (TreeView t in Trees())
        {
            t.ExpandAll();
            t.EndUpdate();
            if (t.Nodes.Count > 0) { t.Nodes[0].EnsureVisible(); }
        }
        fillingTree = false;
        foreach (TreeView t in Trees()) { foreach (TreeNode g in t.Nodes) { SyncGroup(g); } }
        UpdateTotal();
    }

    void AiChecked(object s, TreeViewEventArgs e)
    {
        if (fillingTree) { return; }
        fillingTree = true;
        AiComp a = e.Node.Tag as AiComp;
        if (a != null && a.Installed) { e.Node.Checked = true; } // Installiertes bleibt
        if (a == null) { foreach (TreeNode n in e.Node.Nodes) { AiComp x = n.Tag as AiComp; if (x != null && !x.Installed) { n.Checked = e.Node.Checked; } } }
        else if (e.Node.Parent != null) { SyncGroup(e.Node.Parent); }
        fillingTree = false;
        UpdateTotal();
    }

    void SyncGroup(TreeNode g)
    {
        bool all = g.Nodes.Count > 0;
        foreach (TreeNode n in g.Nodes) { if (!n.Checked) { all = false; } }
        bool was = fillingTree; fillingTree = true; g.Checked = all; fillingTree = was;
    }

    void SetAi(bool recommended)
    {
        fillingTree = true;
        foreach (TreeView t in Trees())
        {
            foreach (TreeNode g in t.Nodes)
            {
                foreach (TreeNode n in g.Nodes) { AiComp a = n.Tag as AiComp; if (a != null && !a.Installed) { n.Checked = recommended && a.Recommended; } }
                SyncGroup(g);
            }
        }
        fillingTree = false;
        UpdateTotal();
    }

    List<AiComp> ChosenAi()
    {
        List<AiComp> v = new List<AiComp>();
        foreach (TreeView t in Trees()) { foreach (TreeNode g in t.Nodes) { foreach (TreeNode n in g.Nodes) { AiComp a = n.Tag as AiComp; if (a != null && n.Checked && !a.Installed) { v.Add(a); } } } }
        return v;
    }

    void UpdateTotal()
    {
        long mb = 0;
        foreach (AiComp a in ChosenAi()) { mb += a.Mb; }
        string appdata = Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData);
        long free = Setup.FreeBytes(appdata);
        aiTotal.Text = (mb > 0 ? "Download: " + Mb(mb) : "Keine Komponente ausgewählt") + (free >= 0 ? "     Frei: " + Gb(free) : "");
        aiTotal.ForeColor = (free >= 0 && mb * 1048576L > free) ? Color.Firebrick : Color.Black;
    }

    // ── Navigation ──

    void Go(int p)
    {
        if (p < 0 || p >= pages.Length) { return; }
        page = p;
        for (int i = 0; i < pages.Length; i++) { pages[i].Visible = i == p; }
        title.Text = titles[p];
        subtitle.Text = subs[p];
        bool side = p == 0 || p == 6;
        banner.Visible = side;
        header.Visible = !side;
        body.BackColor = side ? Color.White : Color.FromArgb(250, 250, 250);
        foreach (Panel pg in pages) { pg.BackColor = body.BackColor; }
        body.Padding = side ? new Padding(32, 28, 28, 12) : new Padding(28, 20, 28, 12);
        back.Enabled = p > 0 && p < 5;
        back.Visible = p < 5;
        next.Text = p == 4 ? "Installieren" : (p == 6 ? "Fertigstellen" : "Weiter >");
        next.Enabled = p != 5;
        cancel.Visible = p != 6;
        if (p == 1) { UpdateSpace(); }
        if (p == 3) { LoadAi(); }
        if (p == 4) { BuildSummary(); }
        if (p == 6) { cLaunchDone.Checked = cLaunch.Checked; }
    }

    void Next()
    {
        if (page == 1)
        {
            string d = dirBox.Text.Trim();
            if (d.Length < 4 || !Path.IsPathRooted(d)) { MessageBox.Show(this, "Bitte einen vollständigen Ordnerpfad angeben.", "Foldex", MessageBoxButtons.OK, MessageBoxIcon.Warning); return; }
            try
            {
                Directory.CreateDirectory(d);
                string probe = Path.Combine(d, ".foldex-probe");
                File.WriteAllText(probe, "x");
                File.Delete(probe);
            }
            catch (Exception)
            {
                MessageBox.Show(this, "In diesen Ordner darf nicht geschrieben werden (z. B. „Programme“ braucht Administratorrechte). Bitte einen anderen wählen.", "Foldex", MessageBoxButtons.OK, MessageBoxIcon.Warning);
                return;
            }
        }
        if (page == 3)
        {
            long mb = 0;
            foreach (AiComp a in ChosenAi()) { mb += a.Mb; }
            long free = Setup.FreeBytes(Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData));
            if (free >= 0 && mb * 1048576L > free)
            {
                if (MessageBox.Show(this, "Für die gewählten KI-Komponenten reicht der freie Speicher vermutlich nicht. Trotzdem fortfahren?", "Foldex", MessageBoxButtons.YesNo, MessageBoxIcon.Warning) != DialogResult.Yes) { return; }
            }
        }
        if (page == 4) { StartInstall(); return; }
        if (page == 6) { Finish(); return; }
        Go(page + 1);
    }

    void BuildSummary()
    {
        StringBuilder s = new StringBuilder();
        s.AppendLine("Installationsordner:");
        s.AppendLine("      " + dirBox.Text);
        s.AppendLine();
        s.AppendLine("Optionen:");
        if (cStart.Checked) { s.AppendLine("      Eintrag im Startmenü"); }
        if (cDesktop.Checked) { s.AppendLine("      Verknüpfung auf dem Desktop"); }
        s.AppendLine("      Eintrag unter „Apps“ (zum Deinstallieren)");
        s.AppendLine();
        List<AiComp> ai = ChosenAi();
        long mb = 0;
        s.AppendLine(ai.Count == 0 ? "Komponenten: keine (später in Foldex hinzufügbar)" : "Komponenten (werden aus dem Internet geladen):");
        foreach (AiComp a in ai) { s.AppendLine("      " + a.Name + "  (" + a.Group + ", " + Mb(a.Mb) + ")"); mb += a.Mb; }
        if (mb > 0) { s.AppendLine("      Zusammen: " + Mb(mb)); }
        summary.Text = s.ToString();
    }

    void Log(string t)
    {
        log.AppendText(DateTime.Now.ToString("HH:mm:ss") + "  " + t + "\r\n");
    }

    void StartInstall()
    {
        installing = true;
        Go(5);
        string dir = dirBox.Text.Trim();
        bool desktop = cDesktop.Checked, start = cStart.Checked;
        List<AiComp> ai = ChosenAi();
        Thread t = new Thread(delegate ()
        {
            string fail = null, aiErr = null;
            try
            {
                double share = ai.Count > 0 ? 0.15 : 1.0; // mit KI: Programm 15 %, KI 85 %
                string last = null;
                Setup.Install(dir, desktop, start, Ver.Value, delegate (double f, string txt)
                {
                    BeginInvoke((Action)delegate
                    {
                        prog.Value = (int)(f * share * 1000);
                        progLabel.Text = txt;
                        if (txt != last) { Log(txt); last = txt; }
                    });
                });
                if (ai.Count > 0)
                {
                    List<string> ids = new List<string>();
                    foreach (AiComp a in ai) { ids.Add(a.Id); }
                    BeginInvoke((Action)delegate { Log("Komponenten werden geladen: " + ai.Count); });
                    string lastItem = null;
                    aiErr = Setup.InstallAi(Path.Combine(dir, Setup.Exe), ids, delegate (double pct, string txt)
                    {
                        BeginInvoke((Action)delegate
                        {
                            prog.Value = Math.Min(1000, (int)((share + (1 - share) * pct / 100.0) * 1000));
                            progLabel.Text = txt;
                            string item = txt.Split('–')[0].Trim();
                            if (item.Length > 0 && item != lastItem && item != "fertig") { Log("Lade " + item); lastItem = item; }
                        });
                    }, delegate { return cancelRequested; });
                }
            }
            catch (Exception ex) { fail = ex.Message; }
            BeginInvoke((Action)delegate { Finished(fail, aiErr); });
        });
        t.IsBackground = true;
        t.Start();
    }

    void Finished(string fail, string aiErr)
    {
        done = true;
        installing = false;
        if (fail != null)
        {
            Log("Fehler: " + fail);
            MessageBox.Show(this, "Die Installation ist fehlgeschlagen:\r\n" + fail, "Foldex", MessageBoxButtons.OK, MessageBoxIcon.Error);
            next.Enabled = false;
            cancel.Text = "Schließen";
            cancel.Visible = true;
            return;
        }
        prog.Value = 1000;
        Log("Fertig");
        string t = "Foldex " + Ver.Value + " wurde erfolgreich installiert.\r\n\r\n";
        if (aiErr == "abgebrochen") { t += "Das Laden der Komponenten wurde abgebrochen. Fehlende Teile lassen sich in Foldex unter Einstellungen → „Lokale Modelle“ hinzufügen.\r\n\r\n"; }
        else if (aiErr != null) { t += "Nicht alle Komponenten ließen sich laden:\r\n" + aiErr + "\r\n\r\n"; }
        t += "Klicke auf „Fertigstellen“, um das Setup zu beenden.";
        doneTitle.Text = "Foldex ist installiert";
        doneLabel.Text = t;
        Go(6);
    }

    void Cancel()
    {
        if (installing && !done)
        {
            if (MessageBox.Show(this, "Installation abbrechen?", "Foldex", MessageBoxButtons.YesNo, MessageBoxIcon.Question) == DialogResult.Yes) { cancelRequested = true; }
            return;
        }
        if (done) { Close(); return; }
        if (MessageBox.Show(this, "Installation abbrechen? Foldex wird dann nicht installiert.", "Foldex", MessageBoxButtons.YesNo, MessageBoxIcon.Question) == DialogResult.Yes) { Close(); }
    }

    void Finish()
    {
        if (cLaunchDone.Checked)
        {
            try { Process.Start(new ProcessStartInfo(Path.Combine(dirBox.Text, Setup.Exe)) { WorkingDirectory = dirBox.Text }); } catch (Exception) { }
        }
        Close();
    }
}
