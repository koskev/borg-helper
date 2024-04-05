# Borg Helper

This is just a little program that helps me creating my borg backups.
It pulls the data from various sources and adds them to the specified repo. Take a look at `config.yaml.example` for an example configuration.

If you add a tag to a repository, only source with the same tag will be added to this repo. This is helpful if you have a remote repo with a storage limitation and only want to backup really important data to that repo.
If a repo is untagged, all data will be backed up to that repo.

Friendly reminder: Since YAML is a superset of JSON, you can also use JSON in the config.

## Backends
It supports various backends for getting the data

### Local
Just a local folder on your pc

```yaml
- name: My PC
  type: local
  folders:
  - /etc
  - path: /home/important
    tags:
      - important
```

### SSH
Get data from a SSH connection

```yaml
- name: remote
  type: ssh
  target: user@server.de
  folders:
    - /root
    - /etc
    - path: /very/large/slow/folder
      options:
        # On slow drives with lots of files the size evaluation can be very slow. This ignores the specified directory
        skip_size: true
```


### Postgres
Get data from a postgres database by using `pg_dumpall`. You can optionally specify a k8s deployment to backup a database running in a kubernetes cluster.

```yaml
 - name: k8s_psql
   type: psql
   user: postgres
   password: mycoolpsqlpassword
   port: 5432
   # Optional: Needs permissions to port forward the deployment
   k8s_deployment: postgresql-deployment-0
   tags:
     - important
```

### Ludusavi
Backup game saves using Ludusavi

```yaml
- name: games
  types: saves
  games:
    # Game specific settings. Currently name is a 1:1 match of the output from ludusavi backup --preview --api
    - name: Stardew Valley
      tags:
        - important
```


