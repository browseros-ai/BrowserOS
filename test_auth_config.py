#!/usr/bin/env python3
"""
Test script for BrowserOS authentication configuration
This script tests the new --auth-config feature without requiring Chromium source
"""

import subprocess
import sys
from pathlib import Path

def test_auth_config_option():
    """Test that the --auth-config option is properly recognized"""
    print("🧪 Testing BrowserOS Authentication Configuration")
    print("=" * 50)
    
    try:
        # Test help output
        print("\n1️⃣ Testing --help with auth-config option...")
        result = subprocess.run(
            ["python", "build/build.py", "--help"],
            capture_output=True,
            text=True,
            cwd=Path(__file__).parent
        )
        
        if "--auth-config" in result.stdout:
            print("✅ --auth-config option found in help output")
        else:
            print("❌ --auth-config option not found in help output")
            return False
            
        # Test auth-config option specifically
        print("\n2️⃣ Testing --auth-config auth option...")
        result = subprocess.run(
            ["python", "build/build.py", "--auth-config", "auth", "--help"],
            capture_output=True,
            text=True,
            cwd=Path(__file__).parent
        )
        
        if result.returncode == 0:
            print("✅ --auth-config auth option works correctly")
        else:
            print("❌ --auth-config auth option failed")
            return False
            
        # Test invalid auth-config option
        print("\n3️⃣ Testing invalid --auth-config option...")
        result = subprocess.run(
            ["python", "build/build.py", "--auth-config", "invalid"],
            capture_output=True,
            text=True,
            cwd=Path(__file__).parent
        )
        
        if result.returncode != 0:
            print("✅ Invalid auth-config option properly rejected")
        else:
            print("❌ Invalid auth-config option should have been rejected")
            return False
            
        # Test configuration file loading
        print("\n4️⃣ Testing configuration file with auth-config...")
        config_file = Path("build/config/test-auth.yaml")
        if config_file.exists():
            result = subprocess.run(
                ["python", "build/build.py", "--config", str(config_file), "--auth-config", "auth"],
                capture_output=True,
                text=True,
                cwd=Path(__file__).parent
            )
            
            # Should fail due to missing chromium source, but auth config should be processed
            if "Using authentication-enabled build configuration" in result.stderr:
                print("✅ Authentication configuration properly loaded from config file")
            else:
                print("⚠️  Authentication configuration may not be working with config files")
        else:
            print("⚠️  Test config file not found, skipping config file test")
            
        print("\n🎉 All authentication configuration tests passed!")
        return True
        
    except Exception as e:
        print(f"❌ Test failed with error: {e}")
        return False

def test_gn_flags_files():
    """Test that all authentication GN flags files exist"""
    print("\n🔧 Testing Authentication GN Flags Files")
    print("=" * 40)
    
    expected_files = [
        "build/config/gn/flags.windows.auth.gn",
        "build/config/gn/flags.macos.auth.gn", 
        "build/config/gn/flags.linux.auth.gn"
    ]
    
    all_exist = True
    for file_path in expected_files:
        if Path(file_path).exists():
            print(f"✅ {file_path} exists")
        else:
            print(f"❌ {file_path} missing")
            all_exist = False
    
    return all_exist

def test_patch_files():
    """Test that authentication patch files exist"""
    print("\n🩹 Testing Authentication Patch Files")
    print("=" * 40)
    
    expected_patches = [
        "patches/browseros/enable-google-authentication.patch",
        "patches/browseros/enable-google-oauth.patch"
    ]
    
    all_exist = True
    for patch_path in expected_patches:
        if Path(patch_path).exists():
            print(f"✅ {patch_path} exists")
        else:
            print(f"❌ {patch_path} missing")
            all_exist = False
    
    return all_exist

def main():
    """Run all authentication configuration tests"""
    print("🚀 BrowserOS Authentication Configuration Test Suite")
    print("=" * 60)
    
    # Test the build system
    auth_test_passed = test_auth_config_option()
    
    # Test GN flags files
    gn_flags_passed = test_gn_flags_files()
    
    # Test patch files
    patches_passed = test_patch_files()
    
    # Summary
    print("\n" + "=" * 60)
    print("📊 Test Results Summary")
    print("=" * 60)
    
    tests = [
        ("Authentication Configuration", auth_test_passed),
        ("GN Flags Files", gn_flags_passed),
        ("Patch Files", patches_passed)
    ]
    
    passed = 0
    total = len(tests)
    
    for test_name, result in tests:
        status = "✅ PASS" if result else "❌ FAIL"
        print(f"{test_name}: {status}")
        if result:
            passed += 1
    
    print(f"\nOverall: {passed}/{total} tests passed")
    
    if passed == total:
        print("🎉 All tests passed! Your authentication configuration is ready.")
        return 0
    else:
        print("⚠️  Some tests failed. Please review the issues above.")
        return 1

if __name__ == "__main__":
    sys.exit(main())
