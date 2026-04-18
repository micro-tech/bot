import { Message } from 'discord.js';
import logger from '../utils/logger';

export const name = 'help';
export const execute = async (message: Message) => {
  try {
    await message.reply('Available commands: hello, ping, help');
    logger.info(`Sent help to ${message.author.tag}`);
  } catch (error) {
    logger.error(`Error sending help: ${error}`);
  }
};