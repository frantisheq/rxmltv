# Running rxmltv as a service

`rxmltv --service` runs without the terminal UI until it is stopped. A service does three things:

- **Updates the guide** every day at the times in `update_times` (local time of the machine; the default is `04:00`).
  An update that was missed, because the computer was off at that time, is made up as soon as the service starts.
- **Shares the guide over HTTP** if `service_serve = true` (`serve_port`, `serve_bind`), see the main README.
- **Sends the notifications** of the notification list through [Apprise](https://appriseit.com/), also when
  nobody is logged in (`notify_apprise = true`). Popups and sounds belong to the terminal UI; the service only
  sends Apprise notifications.

It does not put itself in the background: the init system (systemd, launchd, rc.d, Windows) starts it and watches it.
It stops cleanly on SIGTERM or Ctrl+C. Messages go to standard error with a time stamp (the journal, a log file).
The settings are read once when it starts, so restart the service after changing `rxmltv.conf`. The notification
list is read again every few seconds, so notifications you add in the terminal UI take effect at once.

The service uses a **user's home folder** for everything: settings in `~/.rxmltv/rxmltv.conf`, the cache in
`~/.rxmltv/cache`, the notification list in `~/.rxmltv/rxmltv.notifications`. A service that runs as the user `rxmltv`
with the home `/var/lib/rxmltv` keeps its files in `/var/lib/rxmltv/.rxmltv`. To manage them with the terminal UI, run it
as that same user and home (examples below). `--config <file>` reads the settings from another file.

## 1. Set up the settings

Copy [`packaging/service/rxmltv.conf.example`](../packaging/service/rxmltv.conf.example) to the service user's
`~/.rxmltv/rxmltv.conf` and edit it. The important lines:

```
output = /var/lib/rxmltv/epg.xml.gz     # an absolute path
groups = cz, sk
update_times = 04:30                    # or several: 04:30, 16:00
service_serve = true                    # share over HTTP (no password, no TLS: trusted networks only)
notify_apprise = true
apprise_urls = tgram://BOT_TOKEN/CHAT_ID
```

Or make the file with the terminal UI as the service user (Settings tab, then `s` to save), and press Enter on
"Send a test notification" to try Apprise.

### Notifications with Apprise

1. Install Apprise (it is a Python program): `pip install apprise`, `pipx install apprise`, or your distribution's package
   (`apt install apprise`, `dnf install apprise`, `brew install apprise`, `pkg install py311-apprise` ...). See
   <https://appriseit.com/> for how to build the notification URL for Telegram, e-mail, Discord, Pushover, ntfy and the rest.
2. Put the URLs in the setting `apprise_urls` (several: separated by spaces or commas), or leave it empty and let the
   `apprise_command` carry the configuration, e.g. `apprise --config /etc/apprise.yml --tag tv`. Apprise's own
   configuration files work too.
3. Check it by hand: `apprise -vv -t "Test" -b "Hello" "tgram://BOT_TOKEN/CHAT_ID"`.
4. Make the list in the terminal UI as the **same user and home as the service**: press `b` on a programme in the
   Guide (a dialog asks once or every time, and this channel or all channels). The Notifications tab changes these
   and the minutes before the start later; a title added there is an every-time notification for all channels from
   now on.

The service must be able to run `apprise`: a systemd service with a restricted `PATH` or a Windows service as Local Service may
not find it; give the full path in `apprise_command` (for example `/usr/local/bin/apprise`).

If the terminal UI and the service both run, the first one that notices a programme announces it. The UI still
shows its popup (and plays its sound) for a programme the service already announced.

## 2. Linux: systemd

Both unit files are in [`packaging/service/systemd`](../packaging/service/systemd). The `.deb` and `.rpm` packages install
them (and the user), you only enable the service.

### As a system service, run by its own non-root user

```sh
sudo useradd --system --create-home --home-dir /var/lib/rxmltv --shell /usr/sbin/nologin rxmltv
sudo cp packaging/service/systemd/rxmltv.service /etc/systemd/system/      # the packages put it in /usr/lib/systemd/system
sudo -u rxmltv install -d /var/lib/rxmltv/.rxmltv
sudoedit -u rxmltv /var/lib/rxmltv/.rxmltv/rxmltv.conf                      # see rxmltv.conf.example
sudo systemctl daemon-reload
sudo systemctl enable --now rxmltv
journalctl -u rxmltv -f                                                     # the log
sudo -u rxmltv env HOME=/var/lib/rxmltv rxmltv --tui                        # manage the settings and notifications
sudo systemctl restart rxmltv                                               # after changing the settings
```

The unit runs unprivileged and is hardened (`ProtectSystem=strict`, only `/var/lib/rxmltv` is writable). A port below
1024 for the HTTP server needs `AmbientCapabilities=CAP_NET_BIND_SERVICE` in the unit, or better a port above 1024
and a reverse proxy.

### As a service of your own user (no root at all)

```sh
mkdir -p ~/.config/systemd/user
cp packaging/service/systemd/rxmltv-user.service ~/.config/systemd/user/rxmltv.service
systemctl --user daemon-reload
systemctl --user enable --now rxmltv
journalctl --user -u rxmltv -f
sudo loginctl enable-linger "$USER"      # keep it running after you log out, and start it at boot
```

(`enable-linger` is the only step that needs root. Without it the service runs while you are logged in.)

## 3. macOS: launchd

Files: [`packaging/service/launchd`](../packaging/service/launchd). Edit the paths and `YOURNAME` in them first.
Put the program in `/usr/local/bin` (or change the path; Homebrew on Apple Silicon is `/opt/homebrew/bin`).

**Launch agent** (your user, starts when you log in, no root):

```sh
mkdir -p ~/Library/LaunchAgents ~/Library/Logs
cp packaging/service/launchd/org.rxmltv.agent.plist ~/Library/LaunchAgents/
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/org.rxmltv.agent.plist
tail -f ~/Library/Logs/rxmltv.log
launchctl bootout gui/$(id -u)/org.rxmltv.agent        # stop it
```

**Launch daemon** (starts at boot, before anybody logs in; still runs as your ordinary user through `UserName`):

```sh
sudo cp packaging/service/launchd/org.rxmltv.daemon.plist /Library/LaunchDaemons/
sudo chown root:wheel /Library/LaunchDaemons/org.rxmltv.daemon.plist
sudo launchctl bootstrap system /Library/LaunchDaemons/org.rxmltv.daemon.plist
sudo launchctl bootout system/org.rxmltv.daemon        # stop it
```

Settings: `~/.rxmltv/rxmltv.conf` of that user. After the first start, macOS may ask whether `rxmltv` may accept incoming network
connections (when the HTTP server is on).

## 4. FreeBSD: rc.d

File: [`packaging/service/freebsd/rxmltv`](../packaging/service/freebsd/rxmltv). It uses `daemon(8)`, which restarts the program if it
stops and runs it as an unprivileged user.

```sh
pw useradd rxmltv -d /var/db/rxmltv -m -s /usr/sbin/nologin -c "rxmltv service"
install -m 555 packaging/service/freebsd/rxmltv /usr/local/etc/rc.d/rxmltv
install -d -o rxmltv /var/db/rxmltv/.rxmltv
# put rxmltv.conf in /var/db/rxmltv/.rxmltv/ (owned by rxmltv; see rxmltv.conf.example)
sysrc rxmltv_enable=YES
service rxmltv start
tail -f /var/log/rxmltv.log
su -m rxmltv -c 'env HOME=/var/db/rxmltv rxmltv --tui'      # manage the settings and notifications
```

Settings of the script (`sysrc rxmltv_user=... rxmltv_home=... rxmltv_args=...`) are described at its top.

**Without root** (your own user, started at boot by cron):

```sh
crontab -e
@reboot /usr/sbin/daemon -r -R 30 -P $HOME/.rxmltv/daemon.pid -o $HOME/.rxmltv/service.log /usr/local/bin/rxmltv --service
```

Stop it with `kill $(cat ~/.rxmltv/daemon.pid)`.

## 5. Windows

Windows has no native way to run a console program as a service, so there are two options. Both start `rxmltv.exe --service`,
which reads `%USERPROFILE%\.rxmltv\rxmltv.conf` (or the `HOME` folder you give it).

**Task Scheduler** (your own account, no administrator, no extra software; starts when you log on):

```powershell
schtasks /Create /TN rxmltv /XML packaging\service\windows\rxmltv-task.xml
schtasks /Run /TN rxmltv
schtasks /End /TN rxmltv
```

or without the XML file:

```powershell
$action   = New-ScheduledTaskAction -Execute 'C:\Program Files\rxmltv\rxmltv.exe' -Argument '--service'
$trigger  = New-ScheduledTaskTrigger -AtLogOn
$settings = New-ScheduledTaskSettingsSet -ExecutionTimeLimit ([TimeSpan]::Zero) -RestartCount 999 -RestartInterval (New-TimeSpan -Minutes 1) -StartWhenAvailable -Hidden
Register-ScheduledTask -TaskName rxmltv -Action $action -Trigger $trigger -Settings $settings
```

**A real Windows service with [NSSM](https://nssm.cc/)** (starts at boot, before anybody logs in). Run in an elevated PowerShell:

```powershell
winget install NSSM.NSSM
.\packaging\service\windows\install-service.ps1 -Exe 'C:\Program Files\rxmltv\rxmltv.exe'
```

By default it runs as the built-in *Local Service* account (not an administrator) with its files in `C:\ProgramData\rxmltv`. To run as
a limited user of your own, add `-Account '.\rxmltv-svc' -Password '...' -DataDir 'C:\Users\rxmltv-svc'`. The log is in
`<DataDir>\logs\rxmltv.log`; `nssm restart rxmltv` restarts it; `nssm remove rxmltv confirm` removes it.

To open the HTTP server to other computers, Windows Firewall asks for permission the first time (or add a rule for the port).

## 6. Other init systems

- **OpenRC** (Alpine, Gentoo): [`packaging/service/openrc/rxmltv.initd`](../packaging/service/openrc/rxmltv.initd), runs as a non-root user under
  `supervise-daemon` (instructions at the top of the file).
- **runit**: `/etc/sv/rxmltv/run`:
  ```sh
  #!/bin/sh
  exec 2>&1
  exec chpst -u rxmltv env HOME=/var/lib/rxmltv /usr/bin/rxmltv --service
  ```
- **Docker or anything that keeps a process running**: run `rxmltv --service` in the foreground and mount a folder as the home.

## Troubleshooting

- *Nothing is updated*: look at the first lines of the log: "Service started. Updates at ..." shows the times it understood.
  An empty `update_times` means no scheduled update. The first update runs at once if the guide is older than the last scheduled time.
- *No notifications*: `notify_apprise = true`? Is the notification list in the same home as the service
  (`ls ~/.rxmltv/rxmltv.notifications` for that user)? Does `apprise_command` work for that user (`sudo -u rxmltv apprise --version`)?
  The log shows "Notification failed" with Apprise's own error.
- *The HTTP server does not start*: the log shows "Cannot listen on ...": the port is in use, or below 1024 without the right.
- *Everything stays in English or Czech by accident*: the language comes from `LANG` of the service (often empty, so English); set
  `language = cs` in the settings file.
