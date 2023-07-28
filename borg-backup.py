#!/usr/bin/env python3

from argparse import ArgumentParser, RawTextHelpFormatter
import subprocess
import socket
import datetime
import pathlib
import getpass
import os
import json

from collections import defaultdict

from typing import List, Dict

def run_cmd(cmd: str, workdir: str = None, print_output = True):
	print("Calling \"{}\" in dir {}".format(cmd, workdir))
	if print_output:
		p = subprocess.Popen([cmd], cwd=workdir, shell=True)
	else:
		p = subprocess.Popen([cmd], cwd=workdir, shell=True, stdout=subprocess.PIPE)
	output = p.communicate()
	retvalue = p.returncode
	return retvalue, output

BACKUP_DATE = datetime.datetime.now(datetime.timezone.utc).astimezone().isoformat()

def get_password(system = None, user = None):
	try:
		import keyring
		password = keyring.get_password(system, user)
		if len(password) == 0:
			raise
		return password
	except:
		return ""


def list_prefix(l, prefix):
	return [prefix + x for x in l]

def list_to_string_prefix(l, prefix):
	return " ".join(list_prefix(l, prefix))

def backup_create(options: str, repo: str, name: str, folders: List[str], excludes: List[str]):
	cmd = "borg create {options} {repo}::{name} {folders} {excludes} --exclude-if-present .nobackup".format(options=options, repo=repo, name=name, folders=" ".join(folders), excludes=list_to_string_prefix(excludes, "--exclude "))
	run_cmd(cmd)

def backup_create_remote(options: str, repo: str, name: str, folders_remote: Dict[str, List[str]], excludes: List[str]):
	for host, folders in folders_remote.items():
		TMP_DIR="/tmp/backup/{}".format(host)
		user, hostname = host.split("@")
		pathlib.Path(TMP_DIR).mkdir(parents=True, exist_ok=True)
		cmd = "sshfs {}:/ {}".format(host, TMP_DIR)
		retval, output = run_cmd(cmd)
		if retval == 0:
			folders_local = list_prefix(folders, TMP_DIR)
			backup_create("--files-cache ctime,size {}".format(options), repo, "{}-{}".format(hostname, name), folders_local, excludes)
			cmd = "fusermount -u {}".format(TMP_DIR)
			run_cmd(cmd)
			backup_prune(repo, prefix=hostname)

def backup_prune(repo: str, keep_daily: int = 7, keep_weekly:int = 4, keep_monthly: int = 6, keep_yearly: int = 0, prefix = ""):
	cmd = "borg prune --list --stats -v {} --keep-daily={} --keep-weekly={} --keep-monthly={} --keep-yearly={} --prefix \"{}\"".format(repo, keep_daily, keep_weekly, keep_monthly, keep_yearly, prefix)
	run_cmd(cmd)

def borg_test_password(repo: str):
	cmd = "borg info {}".format(repo)
	retcode, output = run_cmd(cmd, print_output=False)
	return retcode == 0


def load_config(config_file):
	with open(config_file, "r") as f:
		input_json = json.loads(f.read())
		return defaultdict(str, input_json)
	return None

def main():
	config = load_config("config.json")

	try:
		password = get_password(config["password"]["system"], config["password"]["user"])
	except:
		password = ""
	os.environ["BORG_PASSPHRASE"] = password
	for repo in config["repositories"]:
		if pathlib.Path(repo).exists() == False:
			print("{} does not exit! Skipping this repo.".format(repo))
			continue

		while not borg_test_password(repo):
			password = getpass.getpass("Enter Password for repo {}:".format(repo))
			os.environ["BORG_PASSPHRASE"] = password

		if borg_test_password(repo):
			backup_create(config["options"], repo, "{}-{}".format(socket.gethostname(), BACKUP_DATE), config["backup_folders"], config["excludes"])
			backup_prune(repo, prefix=socket.gethostname())
			backup_create_remote(config["options"], repo, BACKUP_DATE, config["remote_folders"], config["excludes"])
		else:
			print("Skipping {}".format(repo))


if __name__ == "__main__":
	main()
