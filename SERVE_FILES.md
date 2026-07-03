# Condividere file via HTTP temporaneo

Per esporre un file (es. un `.deb` appena compilato) su una porta accessibile dall'esterno:

```bash
# 1. Entra nella directory che contiene il file da servire
cd /home/ubuntu/temp-projects/BrowserOS

# 2. Avvia il server HTTP in background sulla porta 55667
nohup python3 -m http.server 55667 > /tmp/http-serve.log 2>&1 &
echo "PID: $!"

# 3. Recupera l'IP pubblico
curl -s https://api.ipify.org

# Il file sarà scaricabile all'indirizzo:
#   http://<IP>:55667/<nome-file>
# Esempio:
#   http://57.129.100.75:55667/BrowserOS.deb
```

## Fermare il server

```bash
# Con il PID stampato al passo 2
kill <PID>

# Oppure, per trovarlo al volo
pkill -f "http.server 55667"
```

## Note

- Il server espone **tutti i file** della directory corrente — avvialo solo dalla cartella giusta.
- Usalo solo per trasferimenti temporanei e chiudilo subito dopo.
- La porta 55667 deve essere aperta nel firewall/security group dell'istanza.
