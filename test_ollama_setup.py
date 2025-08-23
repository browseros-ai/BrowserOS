#!/usr/bin/env python3
"""
Ollama Setup Test Script for BrowserOS
This script helps diagnose and fix common Ollama 403 errors
"""

import requests
import json
import sys
import subprocess
import platform
from pathlib import Path

class OllamaTester:
    def __init__(self, base_url="http://localhost:11434"):
        self.base_url = base_url
        self.session = requests.Session()
        
    def print_header(self, title):
        print(f"\n{'='*60}")
        print(f"🔍 {title}")
        print(f"{'='*60}")
        
    def print_success(self, message):
        print(f"✅ {message}")
        
    def print_error(self, message):
        print(f"❌ {message}")
        
    def print_warning(self, message):
        print(f"⚠️  {message}")
        
    def print_info(self, message):
        print(f"ℹ️  {message}")
        
    def test_basic_connectivity(self):
        """Test if Ollama service is accessible"""
        self.print_header("Testing Basic Connectivity")
        
        try:
            response = self.session.get(f"{self.base_url}/api/tags", timeout=5)
            
            if response.status_code == 200:
                self.print_success("Ollama service is accessible")
                return True
            elif response.status_code == 403:
                self.print_error("Access forbidden (403) - This is the error you're experiencing!")
                self.print_info("This usually indicates a CORS or network configuration issue")
                return False
            else:
                self.print_error(f"Unexpected status code: {response.status_code}")
                return False
                
        except requests.exceptions.ConnectionError:
            self.print_error("Cannot connect to Ollama service")
            self.print_info("Make sure Ollama is running with: ollama serve")
            return False
        except requests.exceptions.Timeout:
            self.print_error("Connection timeout")
            self.print_info("Check if Ollama is running and accessible")
            return False
        except Exception as e:
            self.print_error(f"Connection failed: {e}")
            return False
            
    def test_api_endpoints(self):
        """Test various Ollama API endpoints"""
        self.print_header("Testing API Endpoints")
        
        endpoints = [
            ("/api/tags", "GET", "List available models"),
            ("/api/generate", "POST", "Generate text (requires model)"),
            ("/api/chat", "POST", "Chat endpoint (requires model)")
        ]
        
        for endpoint, method, description in endpoints:
            try:
                if method == "GET":
                    response = self.session.get(f"{self.base_url}{endpoint}", timeout=5)
                else:
                    # For POST endpoints, send minimal data
                    data = {"model": "test", "prompt": "test"} if "generate" in endpoint else {"model": "test"}
                    response = self.session.post(f"{self.base_url}{endpoint}", 
                                               json=data, timeout=5)
                
                if response.status_code == 200:
                    self.print_success(f"{endpoint}: {description}")
                elif response.status_code == 403:
                    self.print_error(f"{endpoint}: Access forbidden (403)")
                elif response.status_code == 400:
                    self.print_warning(f"{endpoint}: Bad request (expected for test data)")
                else:
                    self.print_error(f"{endpoint}: Status {response.status_code}")
                    
            except Exception as e:
                self.print_error(f"{endpoint}: {str(e)}")
                
    def test_models(self):
        """Test if models are available"""
        self.print_header("Testing Available Models")
        
        try:
            response = self.session.get(f"{self.base_url}/api/tags", timeout=5)
            
            if response.status_code == 200:
                models = response.json().get("models", [])
                
                if models:
                    self.print_success(f"Found {len(models)} model(s):")
                    for model in models[:5]:  # Show first 5 models
                        print(f"  - {model.get('name', 'Unknown')}")
                    if len(models) > 5:
                        print(f"  ... and {len(models) - 5} more")
                else:
                    self.print_warning("No models found")
                    self.print_info("Download a model with: ollama pull llama2")
                    
            else:
                self.print_error(f"Failed to get models: {response.status_code}")
                
        except Exception as e:
            self.print_error(f"Error testing models: {e}")
            
    def test_specific_model(self, model_name="llama2"):
        """Test a specific model"""
        self.print_header(f"Testing Model: {model_name}")
        
        try:
            # Test generation with the model
            data = {
                "model": model_name,
                "prompt": "Hello, this is a test message.",
                "stream": False
            }
            
            response = self.session.post(f"{self.base_url}/api/generate", 
                                       json=data, timeout=30)
            
            if response.status_code == 200:
                result = response.json()
                self.print_success(f"Model {model_name} is working!")
                if "response" in result:
                    print(f"  Response: {result['response'][:100]}...")
            elif response.status_code == 403:
                self.print_error(f"Model {model_name}: Access forbidden (403)")
                self.print_info("Check model permissions and CORS settings")
            elif response.status_code == 400:
                error_msg = response.json().get("error", "Unknown error")
                if "model" in error_msg.lower():
                    self.print_error(f"Model {model_name} not found")
                    self.print_info(f"Download it with: ollama pull {model_name}")
                else:
                    self.print_error(f"Model error: {error_msg}")
            else:
                self.print_error(f"Model test failed: {response.status_code}")
                
        except Exception as e:
            self.print_error(f"Error testing model: {e}")
            
    def check_system_info(self):
        """Check system information for troubleshooting"""
        self.print_header("System Information")
        
        # OS Info
        os_name = platform.system()
        os_version = platform.version()
        print(f"Operating System: {os_name} {os_version}")
        
        # Check if Ollama is in PATH
        try:
            result = subprocess.run(["ollama", "--version"], 
                                  capture_output=True, text=True, timeout=5)
            if result.returncode == 0:
                self.print_success(f"Ollama installed: {result.stdout.strip()}")
            else:
                self.print_error("Ollama not found in PATH")
        except FileNotFoundError:
            self.print_error("Ollama not installed or not in PATH")
        except subprocess.TimeoutExpired:
            self.print_warning("Ollama command timed out")
            
        # Check network ports
        self.check_network_ports()
        
    def check_network_ports(self):
        """Check if port 11434 is accessible"""
        self.print_info("Checking network ports...")
        
        try:
            import socket
            sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            sock.settimeout(2)
            result = sock.connect_ex(('localhost', 11434))
            sock.close()
            
            if result == 0:
                self.print_success("Port 11434 is accessible")
            else:
                self.print_error("Port 11434 is not accessible")
                self.print_info("Make sure Ollama is running with: ollama serve")
                
        except Exception as e:
            self.print_error(f"Error checking port: {e}")
            
    def provide_solutions(self):
        """Provide solutions for common issues"""
        self.print_header("Solutions for 403 Errors")
        
        print("🔧 Common Solutions:")
        print("1. Restart Ollama with CORS-friendly settings:")
        print("   ollama serve --host 0.0.0.0")
        print()
        print("2. Check if Ollama is running:")
        print("   ollama serve")
        print()
        print("3. Verify model exists:")
        print("   ollama list")
        print()
        print("4. Test API manually:")
        print("   curl http://localhost:11434/api/tags")
        print()
        print("5. Check firewall settings:")
        if platform.system() == "Windows":
            print("   - Windows Firewall → Allow Ollama")
        elif platform.system() == "Darwin":  # macOS
            print("   - System Preferences → Security → Firewall")
        else:  # Linux
            print("   - Check ufw or iptables settings")
        print()
        print("6. Try different URL:")
        print("   - http://127.0.0.1:11434")
        print("   - http://0.0.0.0:11434")
        
    def run_full_test(self):
        """Run complete Ollama test suite"""
        self.print_header("Ollama Setup Test Suite for BrowserOS")
        
        print("This script will test your Ollama setup and help diagnose 403 errors.")
        print("Make sure Ollama is running in another terminal with: ollama serve")
        print()
        
        # Run all tests
        connectivity_ok = self.test_basic_connectivity()
        
        if connectivity_ok:
            self.test_api_endpoints()
            self.test_models()
            self.test_specific_model()
        else:
            self.print_warning("Skipping API tests due to connectivity issues")
            
        self.check_system_info()
        self.provide_solutions()
        
        # Summary
        self.print_header("Test Summary")
        if connectivity_ok:
            self.print_success("Ollama appears to be working correctly!")
            self.print_info("If you still get 403 errors in BrowserOS, check CORS settings")
        else:
            self.print_error("Ollama has connectivity issues that need to be resolved")
            self.print_info("Follow the solutions above to fix the problems")
            
        print("\nFor more help, check the Ollama Setup Guide:")
        print("docs/OLLAMA_SETUP_GUIDE.md")

def main():
    """Main function"""
    if len(sys.argv) > 1:
        base_url = sys.argv[1]
    else:
        base_url = "http://localhost:11434"
        
    print("🚀 Ollama Setup Test Script for BrowserOS")
    print("=" * 60)
    
    tester = OllamaTester(base_url)
    tester.run_full_test()

if __name__ == "__main__":
    main()
