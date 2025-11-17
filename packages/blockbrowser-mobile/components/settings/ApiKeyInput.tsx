import React, { useState, useEffect } from 'react';
import {
  View,
  TextInput,
  TouchableOpacity,
  StyleSheet,
  useColorScheme,
  ActivityIndicator,
  Alert,
} from 'react-native';
import { Ionicons } from '@expo/vector-icons';
import { Colors } from '@/constants/Colors';
import { SecureStorage } from '@/lib/storage/secure-store';

interface ApiKeyInputProps {
  providerId: string;
  onSave?: (apiKey: string) => void;
}

export const ApiKeyInput: React.FC<ApiKeyInputProps> = ({
  providerId,
  onSave,
}) => {
  const colorScheme = useColorScheme();
  const colors = colorScheme === 'dark' ? Colors.dark : Colors.light;

  const [apiKey, setApiKey] = useState('');
  const [isSecure, setIsSecure] = useState(true);
  const [isSaving, setIsSaving] = useState(false);
  const [hasExistingKey, setHasExistingKey] = useState(false);

  useEffect(() => {
    loadExistingKey();
  }, [providerId]);

  const loadExistingKey = async () => {
    try {
      const existingKey = await SecureStorage.getApiKey(providerId);
      if (existingKey) {
        setHasExistingKey(true);
        setApiKey('••••••••••••••••'); // Show masked placeholder
      }
    } catch (error) {
      console.error('blockbrowser: Error loading API key:', error);
    }
  };

  const handleSave = async () => {
    if (!apiKey || apiKey === '••••••••••••••••') return;

    setIsSaving(true);
    try {
      await SecureStorage.saveApiKey(providerId, apiKey);
      setHasExistingKey(true);
      Alert.alert('Success', 'API key saved securely');
      onSave?.(apiKey);
      // Mask the key after saving
      setApiKey('••••••••••••••••');
      setIsSecure(true);
    } catch (error) {
      console.error('blockbrowser: Error saving API key:', error);
      Alert.alert('Error', 'Failed to save API key');
    } finally {
      setIsSaving(false);
    }
  };

  const handleDelete = async () => {
    Alert.alert(
      'Delete API Key',
      'Are you sure you want to delete this API key?',
      [
        { text: 'Cancel', style: 'cancel' },
        {
          text: 'Delete',
          style: 'destructive',
          onPress: async () => {
            try {
              await SecureStorage.deleteApiKey(providerId);
              setApiKey('');
              setHasExistingKey(false);
              Alert.alert('Success', 'API key deleted');
            } catch (error) {
              console.error('blockbrowser: Error deleting API key:', error);
              Alert.alert('Error', 'Failed to delete API key');
            }
          },
        },
      ]
    );
  };

  const handleFocus = () => {
    if (hasExistingKey && apiKey === '••••••••••••••••') {
      // Clear placeholder when editing existing key
      setApiKey('');
    }
  };

  return (
    <View style={styles.container}>
      <View
        style={[
          styles.inputContainer,
          {
            backgroundColor: colors.background,
            borderColor: colors.border,
          },
        ]}
      >
        <TextInput
          style={[styles.input, { color: colors.text }]}
          value={apiKey}
          onChangeText={setApiKey}
          onFocus={handleFocus}
          placeholder="Enter API key..."
          placeholderTextColor={colors.textSecondary}
          secureTextEntry={isSecure}
          autoCapitalize="none"
          autoCorrect={false}
          editable={!isSaving}
        />
        <View style={styles.actions}>
          {/* Toggle visibility */}
          <TouchableOpacity
            onPress={() => setIsSecure(!isSecure)}
            style={styles.iconButton}
          >
            <Ionicons
              name={isSecure ? 'eye-off' : 'eye'}
              size={20}
              color={colors.textSecondary}
            />
          </TouchableOpacity>

          {/* Delete existing key */}
          {hasExistingKey && (
            <TouchableOpacity
              onPress={handleDelete}
              style={styles.iconButton}
              disabled={isSaving}
            >
              <Ionicons
                name="trash"
                size={20}
                color={colors.error}
              />
            </TouchableOpacity>
          )}

          {/* Save button */}
          <TouchableOpacity
            onPress={handleSave}
            style={[
              styles.saveButton,
              {
                backgroundColor: apiKey && apiKey !== '••••••••••••••••'
                  ? colors.primary
                  : colors.border,
              },
            ]}
            disabled={!apiKey || apiKey === '••••••••••••••••' || isSaving}
          >
            {isSaving ? (
              <ActivityIndicator size="small" color={colors.background} />
            ) : (
              <Ionicons
                name="checkmark"
                size={20}
                color={
                  apiKey && apiKey !== '••••••••••••••••'
                    ? colors.background
                    : colors.textSecondary
                }
              />
            )}
          </TouchableOpacity>
        </View>
      </View>
    </View>
  );
};

const styles = StyleSheet.create({
  container: {
    width: '100%',
  },
  inputContainer: {
    flexDirection: 'row',
    alignItems: 'center',
    borderRadius: 10,
    borderWidth: 1,
    paddingLeft: 12,
    paddingRight: 4,
    gap: 8,
  },
  input: {
    flex: 1,
    fontSize: 14,
    paddingVertical: 10,
    fontFamily: 'monospace',
  },
  actions: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 4,
  },
  iconButton: {
    padding: 8,
  },
  saveButton: {
    width: 36,
    height: 36,
    borderRadius: 8,
    alignItems: 'center',
    justifyContent: 'center',
  },
});
