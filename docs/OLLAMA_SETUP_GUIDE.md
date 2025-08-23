# 🦙 Ollama Setup Guide for BrowserOS

## 🎯 **Overview**

This guide helps you set up Ollama in BrowserOS and avoid the common **403 Error** that many users encounter. Ollama allows you to run AI models locally on your computer, providing privacy and offline AI capabilities.

## 🚨 **Common 403 Error & Solutions**

### **What is the 403 Error?**
The 403 "Forbidden" error occurs when BrowserOS cannot properly access your local Ollama service. This is usually caused by:
- **CORS (Cross-Origin Resource Sharing)** issues
- **Network configuration** problems
- **Ollama service** not running properly
- **Firewall/security** blocking access

### **Quick Fix Steps:**
1. **Ensure Ollama is running**: `ollama serve`
2. **Check the URL**: Use `http://localhost:11434`
3. **Verify model exists**: `ollama list`
4. **Test connection**: `curl http://localhost:11434/api/tags`

## 🛠️ **Step-by-Step Setup**

### **Step 1: Install Ollama**

#### **Windows:**
```bash
# Download from https://ollama.com/download
# Or use winget:
winget install Ollama.Ollama
```

#### **macOS:**
```bash
# Download from https://ollama.com/download
# Or use Homebrew:
brew install ollama
```

#### **Linux:**
```bash
curl -fsSL https://ollama.com/install.sh | sh
```

### **Step 2: Start Ollama Service**

```bash
# Start Ollama service
ollama serve

# Keep this terminal open - Ollama needs to keep running
```

**Expected Output:**
```
Starting Ollama server...
Listening on 0.0.0.0:11434
```

### **Step 3: Download a Model**

```bash
# Download a popular model (this may take a while)
ollama pull llama2

# Or try a smaller model for faster setup
ollama pull llama2:7b
```

### **Step 4: Verify Ollama is Working**

```bash
# Test the API
curl http://localhost:11434/api/tags

# Expected output: JSON with available models
```

## 🔧 **BrowserOS Configuration**

### **Step 1: Open BrowserOS Settings**
1. Open BrowserOS
2. Go to **Settings** → **AI & LLM Settings**
3. Click **Add Provider**

### **Step 2: Configure Ollama Provider**
- **Provider Type**: Select `Ollama`
- **Name**: Give it a name (e.g., "Local Ollama")
- **Base URL**: `http://localhost:11434`
- **Model**: Enter the model name (e.g., `llama2`)
- **API Key**: Leave empty (not required for local Ollama)

### **Step 3: Test Connection**
Click **Test Connection** to verify everything works.

## 🚨 **Troubleshooting 403 Errors**

### **Error 1: "Access forbidden (403)"**

#### **Solution A: Check Ollama Service**
```bash
# Make sure Ollama is running
ollama serve

# Check if it's listening on the right port
netstat -an | grep 11434
```

#### **Solution B: Fix CORS Issues**
```bash
# Stop Ollama (Ctrl+C)
# Restart with CORS-friendly settings
ollama serve --host 0.0.0.0
```

#### **Solution C: Check Firewall**
- **Windows**: Allow Ollama through Windows Firewall
- **macOS**: Check System Preferences → Security & Privacy → Firewall
- **Linux**: Check `ufw` or `iptables` settings

### **Error 2: "Model not found"**

#### **Solution: Verify Model Name**
```bash
# List available models
ollama list

# Pull the model if it doesn't exist
ollama pull [model-name]
```

### **Error 3: "Connection refused"**

#### **Solution: Check Network Configuration**
```bash
# Test if port is accessible
telnet localhost 11434

# Or use curl
curl -v http://localhost:11434/api/tags
```

## 🔒 **Advanced Configuration**

### **Custom Ollama Setup**

#### **Multiple Models:**
```bash
# Pull different models
ollama pull llama2:7b
ollama pull llama2:13b
ollama pull codellama:7b
ollama pull mistral:7b
```

#### **Custom Model Configuration:**
```bash
# Create a custom model configuration
ollama create mymodel -f Modelfile
```

### **Network Configuration**

#### **Remote Ollama Access:**
If you want to access Ollama from another computer:
```bash
# Start Ollama on specific interface
ollama serve --host 0.0.0.0

# Access from other computers
# http://[your-ip]:11434
```

#### **Docker Setup:**
```bash
# Run Ollama in Docker
docker run -d -v ollama:/root/.ollama -p 11434:11434 --name ollama ollama/ollama

# Test connection
curl http://localhost:11434/api/tags
```

## 🧪 **Testing Your Setup**

### **Test 1: Basic API Access**
```bash
curl http://localhost:11434/api/tags
```

### **Test 2: Model Generation**
```bash
curl -X POST http://localhost:11434/api/generate \
  -H "Content-Type: application/json" \
  -d '{
    "model": "llama2",
    "prompt": "Hello, how are you?",
    "stream": false
  }'
```

### **Test 3: BrowserOS Integration**
1. Open BrowserOS AI Chat
2. Select your Ollama provider
3. Send a test message
4. Verify response is received

## 📱 **Mobile/Remote Access**

### **Access from Mobile Device:**
```bash
# Find your computer's IP address
ipconfig (Windows)
ifconfig (macOS/Linux)

# Start Ollama with network access
ollama serve --host 0.0.0.0

# Access from mobile: http://[your-ip]:11434
```

### **Security Considerations:**
- Only enable network access on trusted networks
- Consider using a reverse proxy with authentication
- Monitor access logs for unauthorized usage

## 🆘 **Getting Help**

### **Common Issues & Solutions:**

#### **Issue: Ollama won't start**
```bash
# Check if port is already in use
lsof -i :11434

# Kill existing process
kill -9 [PID]

# Restart Ollama
ollama serve
```

#### **Issue: Model download fails**
```bash
# Check internet connection
ping 8.8.8.8

# Try different model
ollama pull llama2:7b

# Check disk space
df -h
```

#### **Issue: Slow responses**
```bash
# Check system resources
htop
nvidia-smi  # If using GPU

# Use smaller model
ollama pull llama2:7b
```

### **Support Resources:**
- **Ollama Documentation**: https://ollama.com/docs
- **BrowserOS Discord**: https://discord.gg/YKwjt5vuKr
- **GitHub Issues**: Report bugs and get help

## 🎉 **Success Checklist**

- ✅ Ollama service is running (`ollama serve`)
- ✅ Model is downloaded (`ollama list` shows models)
- ✅ API is accessible (`curl http://localhost:11434/api/tags`)
- ✅ BrowserOS can connect (Test Connection passes)
- ✅ AI chat works (can send/receive messages)

## 🔮 **Next Steps**

Once Ollama is working:
1. **Explore different models** for various tasks
2. **Customize model parameters** for better results
3. **Set up multiple providers** for different use cases
4. **Contribute improvements** to BrowserOS Ollama integration

---

**Need help?** Join the [BrowserOS Discord](https://discord.gg/YKwjt5vuKr) for community support!
