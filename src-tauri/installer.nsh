; Casty has to accept inbound LAN traffic or other devices can never find it.
; Without these rules Windows silently drops the discovery probes and the stream connection,
; which looks exactly like "the other machine cannot see this PC".

!macro NSIS_HOOK_POSTINSTALL
  nsExec::ExecToLog 'netsh advfirewall firewall delete rule name="Casty stream (TCP 45455)"'
  nsExec::ExecToLog 'netsh advfirewall firewall delete rule name="Casty discovery (UDP 45454)"'
  nsExec::ExecToLog 'netsh advfirewall firewall add rule name="Casty stream (TCP 45455)" dir=in action=allow protocol=TCP localport=45455 profile=private,domain'
  nsExec::ExecToLog 'netsh advfirewall firewall add rule name="Casty discovery (UDP 45454)" dir=in action=allow protocol=UDP localport=45454 profile=private,domain'
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  nsExec::ExecToLog 'netsh advfirewall firewall delete rule name="Casty stream (TCP 45455)"'
  nsExec::ExecToLog 'netsh advfirewall firewall delete rule name="Casty discovery (UDP 45454)"'
!macroend
